//! Connections to the printers: live status over MQTT and job dispatch.

pub mod ftp;
pub mod hms;
pub mod mapping;
pub mod status;

use crate::models::{ConnectionSettings, Printer, PrintOptions, now};
use crate::tls::printer_client_config;
use anyhow::{anyhow, bail};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde_json::{Value, json};
use status::{EXTERNAL_TRAY, StatusView};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Notify, RwLock, watch},
    task::JoinHandle,
};

/// The error when a command can't be sent because the printer is offline;
/// nothing reached the printer, so the command can safely be sent again.
pub const NOT_CONNECTED: &str = "printer is not connected";

impl LiveState {
    /// Connected, with an AMS whose model the printer hasn't told yet (an AMS HT's is
    /// known from its id), or no module list at all.
    fn lacks_ams_model(&self) -> bool {
        self.connected && (self.ams_models.is_empty() || status::view(&self.print).ams_units.iter().any(|u| u.id < 128 && !self.ams_models.contains_key(&u.id)))
    }
}

#[derive(Default)]
struct LiveState {
    connected: bool,
    last_error: Option<String>,
    last_update: Option<i64>,
    /// Merged `print` object from all reports.
    print: Value,
    /// The printer's last answer to a command (`command`, `result`, `reason`).
    reply: Option<Value>,
    /// AMS unit id → model, from `get_version`.
    ams_models: HashMap<u32, String>,
}

struct Conn {
    printer: Printer,
    client: Mutex<Option<AsyncClient>>,
    live: RwLock<LiveState>,
    task: Mutex<Option<JoinHandle<()>>>,
    /// Bumped on every report, so callers can wait for changes.
    tick: watch::Sender<u64>,
    /// Cuts a reconnect pause short (someone is waiting for this printer).
    wake: Notify,
}

#[derive(Default)]
pub struct PrinterManager {
    conns: RwLock<HashMap<String, Arc<Conn>>>,
    settings: Arc<std::sync::RwLock<ConnectionSettings>>,
}

fn needs_restart(old: &Printer, new: &Printer) -> bool {
    old.host != new.host || old.serial != new.serial || old.access_code != new.access_code || old.enabled != new.enabled
}

impl PrinterManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn settings(&self) -> ConnectionSettings {
        *self.settings.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_settings(&self, s: ConnectionSettings) {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = s;
    }

    /// Try to reconnect now instead of after the current pause.
    pub async fn wake(&self, id: &str) {
        if let Ok(conn) = self.conn(id).await {
            conn.wake.notify_one();
        }
    }

    /// Wait until the printer reports something new (or `max` passes).
    pub async fn changed(&self, id: &str, max: Duration) {
        let Ok(conn) = self.conn(id).await else {
            tokio::time::sleep(max).await;
            return;
        };
        let mut rx = conn.tick.subscribe();
        let _ = tokio::time::timeout(max, rx.changed()).await;
    }

    /// Bring connections in line with the configured printers.
    pub async fn sync(&self, printers: &[Printer]) {
        let mut conns = self.conns.write().await;
        conns.retain(|id, conn| {
            let keep = printers.iter().any(|p| &p.id == id && !needs_restart(&conn.printer, p));
            if !keep
                && let Ok(mut task) = conn.task.try_lock()
                    && let Some(t) = task.take() {
                        t.abort();
                    }
            keep
        });
        for p in printers {
            if let Some(existing) = conns.get(&p.id) {
                // Name/profile edits don't need a reconnect, but keep the copy fresh.
                if existing.printer.name != p.name || existing.printer.printer_model != p.printer_model {
                    let conn = self.spawn_conn(p.clone(), existing.live.read().await.print.clone());
                    if let Some(t) = existing.task.lock().await.take() {
                        t.abort();
                    }
                    conns.insert(p.id.clone(), conn);
                }
                continue;
            }
            conns.insert(p.id.clone(), self.spawn_conn(p.clone(), Value::Null));
        }
    }

    fn spawn_conn(&self, printer: Printer, print: Value) -> Arc<Conn> {
        let (tick, _) = watch::channel(0);
        let conn = Arc::new(Conn {
            printer: printer.clone(),
            client: Mutex::new(None),
            live: RwLock::new(LiveState { print, ..Default::default() }),
            task: Mutex::new(None),
            tick,
            wake: Notify::new(),
        });
        if printer.enabled {
            let handle = tokio::spawn(run_connection(conn.clone(), self.settings.clone()));
            if let Ok(mut t) = conn.task.try_lock() {
                *t = Some(handle);
            }
        } else if let Ok(mut live) = conn.live.try_write() {
            live.last_error = Some("disabled".into());
        }
        conn
    }

    async fn conn(&self, id: &str) -> anyhow::Result<Arc<Conn>> {
        self.conns.read().await.get(id).cloned().ok_or_else(|| anyhow!("unknown printer"))
    }

    pub async fn status(&self, id: &str) -> StatusView {
        let Ok(conn) = self.conn(id).await else { return StatusView::default() };
        let live = conn.live.read().await;
        let mut v = status::view(&live.print);
        v.ams_units.iter_mut().for_each(|u| u.set_model(&live.ams_models));
        v.connected = live.connected;
        v.last_error = live.last_error.clone();
        v.last_update = live.last_update;
        v
    }

    async fn publish(&self, id: &str, payload: Value) -> anyhow::Result<()> {
        let conn = self.conn(id).await?;
        if !conn.live.read().await.connected {
            bail!(NOT_CONNECTED);
        }
        let client = conn.client.lock().await.clone().ok_or_else(|| anyhow!(NOT_CONNECTED))?;
        let topic = format!("device/{}/request", conn.printer.serial);
        client.publish(topic, QoS::AtMostOnce, false, serde_json::to_vec(&payload)?).await?;
        Ok(())
    }

    pub async fn request_full_status(&self, id: &str) -> anyhow::Result<()> {
        self.publish(id, json!({"pushing": {"sequence_id": "0", "command": "pushall"}})).await
    }

    /// Why the printer refuses unsigned commands (Developer Mode off), when it
    /// does: set the chamber light to what it already is and watch for the
    /// refusal code. `None` also when it can't tell (no light reported).
    pub async fn commands_refused(&self, id: &str) -> Option<&'static str> {
        const REFUSED: &str = "0500_0500_0001_0007";
        let s = self.status(id).await;
        if !s.connected {
            return None;
        }
        if s.hms.iter().any(|c| c == REFUSED) {
            return status::hms_explained(REFUSED);
        }
        let on = s.chamber_light?;
        let conn = self.conn(id).await.ok()?;
        let mut rx = conn.tick.subscribe();
        self.control(id, if on { "light_on" } else { "light_off" }).await.ok()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        loop {
            if self.status(id).await.hms.iter().any(|c| c == REFUSED) {
                return status::hms_explained(REFUSED);
            }
            if !matches!(tokio::time::timeout_at(deadline, rx.changed()).await, Ok(Ok(()))) {
                return None;
            }
        }
    }

    pub async fn control(&self, id: &str, action: &str) -> anyhow::Result<()> {
        let payload = match action {
            "pause" | "resume" | "stop" => json!({"print": {"sequence_id": "0", "command": action, "param": ""}}),
            "light_on" | "light_off" => json!({"system": {
                "sequence_id": "0", "command": "ledctrl", "led_node": "chamber_light",
                "led_mode": if action == "light_on" { "on" } else { "off" },
                "led_on_time": 500, "led_off_time": 500, "loop_times": 0, "interval_time": 0
            }}),
            "refresh" => return self.request_full_status(id).await,
            // 1 silent, 2 standard, 3 sport, 4 ludicrous
            "speed_1" | "speed_2" | "speed_3" | "speed_4" => {
                json!({"print": {"sequence_id": "0", "command": "print_speed", "param": &action[6..]}})
            }
            other => bail!("unknown action {other:?}"),
        };
        self.publish(id, payload).await?;
        if action == "pause" {
            crate::attention::paused_by_us(id);
        }
        Ok(())
    }

    /// Tell the printer what is loaded in a tray, as its own filament menu
    /// does for spools without an RFID tag.
    pub async fn set_tray(&self, id: &str, tray: i32, setting: &TraySetting) -> anyhow::Result<()> {
        self.publish(id, tray_setting_command(tray, setting)).await?;
        self.request_full_status(id).await
    }

    /// Have the AMS read the spool's RFID tag again.
    pub async fn reread_tray(&self, id: &str, tray: i32) -> anyhow::Result<()> {
        self.publish(id, reread_command(tray)?).await
    }

    /// Start drying an AMS unit, or stop it. `Ok(false)` when the unit's report
    /// hasn't changed within 10 s, which doesn't mean it refused. A "success" reply
    /// proves nothing: a P1S sends one and then ignores the command.
    pub async fn set_drying(&self, id: &str, unit: u32, dry: Option<&DrySetting>) -> anyhow::Result<bool> {
        let conn = self.conn(id).await?;
        let mut rx = conn.tick.subscribe();
        let hms_before = self.status(id).await.hms;
        conn.live.write().await.reply = None;
        self.publish(id, drying_command(unit, dry)).await?;

        let wait = Duration::from_secs(10);
        let started = tokio::time::Instant::now();
        let deadline = started + wait;
        let mut asked = false;
        loop {
            let s = self.status(id).await;
            if s.ams_units.iter().any(|u| u.id == unit && u.dry_minutes.is_some() == dry.is_some()) {
                return Ok(true);
            }
            // Other messages can come up in these seconds and say nothing about drying.
            if let Some((code, why)) = s.hms.iter().filter(|c| !hms_before.contains(c)).find_map(|c| status::hms_explained(c).map(|why| (c, why))) {
                bail!("{why} (HMS {code})");
            }
            let reply = conn.live.read().await.reply.clone();
            if let Some(reason) = refusal(reply.as_ref(), DRYING_COMMAND) {
                bail!("the printer refused: {reason}");
            }
            if !asked && started.elapsed() >= wait / 2 {
                asked = true;
                let _ = self.request_full_status(id).await;
            }
            let next = if asked { deadline } else { started + wait / 2 };
            match tokio::time::timeout_at(next, rx.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => bail!("connection to the printer was reset"),
                Err(_) if tokio::time::Instant::now() < deadline => continue,
                Err(_) => return Ok(false),
            }
        }
    }

    /// Send the print command for a file already uploaded to the printer.
    /// Waits briefly for the printer to react so failures are reported.
    pub async fn start_print(
        &self,
        printer: &Printer,
        remote_file: &str,
        gcode_entry: &str,
        ams_mapping: &[i32],
        opts: &PrintOptions,
    ) -> anyhow::Result<()> {
        let status = self.status(&printer.id).await;
        if !status.connected {
            bail!(NOT_CONNECTED);
        }
        if status.is_busy() {
            bail!("printer is busy ({})", status.state.unwrap_or_default());
        }
        // Checked before the upload too; drying may have started since.
        if let Some(why) = status.drying_refusal(ams_mapping) {
            bail!("{why}");
        }
        let h2 = printer.printer_model.to_uppercase().contains(" H2");
        let payload = build_print_command(remote_file, gcode_entry, ams_mapping, opts, h2);
        let conn = self.conn(&printer.id).await?;
        let mut rx = conn.tick.subscribe();
        let error_before = status.print_error;
        let hms_before = status.hms.clone();
        conn.live.write().await.reply = None;
        tracing::info!("sending print command to {}: {}", printer.name, payload);
        self.publish(&printer.id, payload).await?;

        // The printer was idle, so any busy state now means our job started.
        // Watch for that, its answer to the command, or a new error.
        let wait = Duration::from_secs(self.settings().start_timeout_secs as u64);
        let started = tokio::time::Instant::now();
        let deadline = started + wait;
        let mut asked = false;
        loop {
            let s = self.status(&printer.id).await;
            if s.is_busy() {
                return Ok(());
            }
            if let Some(err) = s.print_error.filter(|e| Some(*e) != error_before && *e != 0) {
                bail!("{} reported error {err:08X} — check the printer screen", printer.name);
            }
            if let Some(code) = s.hms.iter().find(|c| !hms_before.contains(c)) {
                match status::hms_explained(code) {
                    Some(why) => bail!("{} didn't start: {why} (HMS {code})", printer.name),
                    None => bail!("{} reported HMS {code} — check the printer screen", printer.name),
                }
            }
            if let Some(reason) = refusal(conn.live.read().await.reply.as_ref(), "project_file") {
                bail!("{} refused the print: {reason}", printer.name);
            }
            // Some firmware only reports on its next periodic push: ask.
            if !asked && started.elapsed() >= wait / 2 {
                asked = true;
                let _ = self.request_full_status(&printer.id).await;
            }
            let next = if asked { deadline } else { deadline.min(started + wait / 2) };
            match tokio::time::timeout_at(next, rx.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => bail!("connection to the printer was reset"),
                Err(_) if tokio::time::Instant::now() < deadline => continue,
                Err(_) => bail!(
                    "{} got the print but didn't start it within {} s. Check the printer screen, then try again.",
                    printer.name,
                    wait.as_secs()
                ),
            }
        }
    }
}

/// What is loaded in a tray.
#[derive(Debug, Clone, PartialEq)]
pub struct TraySetting {
    /// Material, e.g. "PLA".
    pub kind: String,
    /// `#RRGGBB`
    pub color: String,
    /// Bambu filament id, e.g. "GFA00" (Bambu PLA Basic) or "GFL99" (Generic PLA).
    pub info_idx: String,
    pub temp_min: u32,
    pub temp_max: u32,
}

/// (ams_id, slot) as the firmware addresses a global tray id; the external
/// spool is ams 255, slot 254, and an AMS HT's one slot has its unit's id.
fn tray_address(tray: i32) -> (i32, i32) {
    match tray {
        EXTERNAL_TRAY => (255, EXTERNAL_TRAY),
        t if t >= 128 => (t, 0),
        t => (t / 4, t % 4),
    }
}

pub fn tray_setting_command(tray: i32, s: &TraySetting) -> Value {
    let (ams, slot) = tray_address(tray);
    json!({"print": {
        "sequence_id": "0",
        "command": "ams_filament_setting",
        "ams_id": ams,
        "tray_id": slot,
        "slot_id": slot,
        "tray_info_idx": s.info_idx,
        "tray_color": format!("{}FF", s.color.trim_start_matches('#').to_uppercase()),
        "nozzle_temp_min": s.temp_min,
        "nozzle_temp_max": s.temp_max,
        "tray_type": s.kind,
        "setting_id": "",
    }})
}

pub fn reread_command(tray: i32) -> anyhow::Result<Value> {
    if tray == EXTERNAL_TRAY {
        bail!("the external spool has no RFID reader");
    }
    let (ams, slot) = tray_address(tray);
    Ok(json!({"print": {"sequence_id": "0", "command": "ams_get_rfid", "ams_id": ams, "slot_id": slot}}))
}

/// What to dry with: temperature in °C, hours, and whether to turn the spools.
#[derive(Debug, Clone, PartialEq)]
pub struct DrySetting {
    pub temp: u32,
    pub hours: u32,
    pub rotate: bool,
}

const DRYING_COMMAND: &str = "ams_filament_drying";
const VERSION_RETRIES: u32 = 8;
const VERSION_RETRY_EVERY: Duration = Duration::from_secs(15);

/// Printers that answer `ams_filament_drying` with "success" and then do nothing.
/// They can dry from their own screen, and report it like any other.
pub fn ignores_drying_command(printer_model: &str, machine_profile: &str) -> bool {
    [printer_model, machine_profile].iter().any(|name| name.split_whitespace().any(|w| w.eq_ignore_ascii_case("P1S") || w.eq_ignore_ascii_case("P1P")))
}

/// `mode` 1 starts and 0 stops. The printer ignores a `cooling_temp` under 45.
pub fn drying_command(unit: u32, dry: Option<&DrySetting>) -> Value {
    json!({"print": {
        "sequence_id": "0",
        "command": DRYING_COMMAND,
        "ams_id": unit,
        "temp": dry.map_or(0, |d| d.temp),
        "cooling_temp": dry.map_or(0, |_| 45),
        "duration": dry.map_or(0, |d| d.hours),
        "humidity": 0,
        "mode": i32::from(dry.is_some()),
        "rotate_tray": dry.is_some_and(|d| d.rotate),
        "filament": "",
        "close_power_conflict": false,
    }})
}

/// The reason when the answer to `command` says it failed.
fn refusal(reply: Option<&Value>, command: &str) -> Option<String> {
    let r = reply?;
    if r.get("command").and_then(|c| c.as_str()) != Some(command) {
        return None;
    }
    let result = r.get("result").and_then(|v| v.as_str()).unwrap_or("");
    if result.eq_ignore_ascii_case("success") || result.is_empty() {
        return None;
    }
    let reason = r.get("reason").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(result);
    Some(reason.to_string())
}

/// The print command for a file uploaded to the printer's SD card, in the
/// form Bambu Studio and the Home Assistant integration use on the LAN:
/// `file:///sdcard/…` (X1, P1, A1) or `ftp:///…` (H2), and ids of "0" (a
/// real id makes the printer look the task up in the cloud).
pub fn build_print_command(remote_file: &str, gcode_entry: &str, ams_mapping: &[i32], opts: &PrintOptions, h2: bool) -> Value {
    // Bambu Studio sends -1 for the external spool in the flat mapping and
    // spells it out in ams_mapping2.
    let mut flat = Vec::new();
    let mut detailed = Vec::new();
    for &t in ams_mapping {
        match t {
            t if t < 0 => {
                flat.push(-1);
                detailed.push(json!({"ams_id": 255, "slot_id": 255}));
            }
            t if t >= EXTERNAL_TRAY => {
                flat.push(-1);
                detailed.push(json!({"ams_id": 255, "slot_id": 0}));
            }
            t if t >= 128 => {
                flat.push(t);
                detailed.push(json!({"ams_id": t, "slot_id": 0}));
            }
            t => {
                flat.push(t);
                detailed.push(json!({"ams_id": t / 4, "slot_id": t % 4}));
            }
        }
    }
    let use_ams = ams_mapping.iter().any(|&t| (0..EXTERNAL_TRAY).contains(&t));
    let subtask = remote_file.trim_end_matches(".3mf").trim_end_matches(".gcode");

    json!({
        "print": {
            "sequence_id": "20000",
            "command": "project_file",
            "param": gcode_entry,
            "url": if h2 { format!("ftp:///{remote_file}") } else { format!("file:///sdcard/{remote_file}") },
            "file": remote_file,
            "md5": "",
            "bed_type": "auto",
            "timelapse": opts.timelapse,
            "bed_leveling": opts.bed_leveling,
            "auto_bed_leveling": if opts.bed_leveling { 1 } else { 0 },
            "flow_cali": opts.flow_cali,
            "extrude_cali_flag": if opts.flow_cali { 1 } else { 0 },
            "extrude_cali_manual_mode": 0,
            "vibration_cali": opts.vibration_cali,
            "layer_inspect": false,
            "use_ams": use_ams,
            "ams_mapping": flat,
            "ams_mapping2": detailed,
            "subtask_name": subtask,
            "profile_id": "0",
            "project_id": "0",
            "subtask_id": "0",
            "task_id": "0",
        }
    })
}

async fn run_connection(conn: Arc<Conn>, settings: Arc<std::sync::RwLock<ConnectionSettings>>) {
    let p = conn.printer.clone();
    let tls = Arc::new(printer_client_config());
    let client_id = format!("airslice-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let mut opts = MqttOptions::new(client_id, p.host.clone(), 8883);
    opts.set_credentials("bblp", p.access_code.clone());
    opts.set_keep_alive(Duration::from_secs(30));
    opts.set_max_packet_size(8 * 1024 * 1024, 256 * 1024);
    opts.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(tls)));
    let (client, mut eventloop) = AsyncClient::new(opts, 32);
    *conn.client.lock().await = Some(client.clone());
    let report_topic = format!("device/{}/report", p.serial);
    const FIRST: Duration = Duration::from_secs(2);
    let mut backoff = FIRST;

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                tracing::info!("connected to printer {} ({})", p.name, p.host);
                backoff = FIRST;
                {
                    let mut live = conn.live.write().await;
                    live.connected = true;
                    live.last_error = None;
                }
                let _ = client.subscribe(report_topic.clone(), QoS::AtMostOnce).await;
                let req = format!("device/{}/request", p.serial);
                let push = json!({"pushing": {"sequence_id": "0", "command": "pushall"}});
                let _ = client.publish(req.clone(), QoS::AtMostOnce, false, push.to_string()).await;
                let version = json!({"info": {"sequence_id": "0", "command": "get_version"}});
                let _ = client.publish(req.clone(), QoS::AtMostOnce, false, version.to_string()).await;
                conn.tick.send_modify(|t| *t += 1);
                // A printer that just booted can miss that request; ask once
                // more if no report has come in after a few seconds.
                let (c, cl, since) = (conn.clone(), client.clone(), now());
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let quiet = { c.live.read().await.last_update.is_none_or(|t| t < since) };
                    if quiet && c.live.read().await.connected {
                        let _ = cl.publish(req.clone(), QoS::AtMostOnce, false, push.to_string()).await;
                    }
                    // AMS units keep turning up for a while after a boot, and the printer
                    // only lists the ones it knows when asked.
                    for _ in 0..VERSION_RETRIES {
                        if !c.live.read().await.lacks_ams_model() {
                            break;
                        }
                        let _ = cl.publish(req.clone(), QoS::AtMostOnce, false, version.to_string()).await;
                        tokio::time::sleep(VERSION_RETRY_EVERY).await;
                    }
                });
            }
            Ok(Event::Incoming(Packet::Publish(msg))) => {
                let Ok(value) = serde_json::from_slice::<Value>(&msg.payload) else { continue };
                if let Some(modules) = value.get("info").and_then(|i| i.get("module")).and_then(|m| m.as_array()) {
                    let mut live = conn.live.write().await;
                    for (id, model) in modules.iter().filter_map(|m| status::ams_module(m.get("name")?.as_str()?)) {
                        live.ams_models.insert(id, model.to_string());
                    }
                    drop(live);
                    conn.tick.send_modify(|t| *t += 1);
                }
                if let Some(print) = value.get("print") {
                    // Answers to commands carry `result` (and `reason` when refused).
                    if let Some(cmd) = print.get("command").and_then(|c| c.as_str()).filter(|c| *c != "push_status")
                        && print.get("result").is_some()
                    {
                        tracing::info!("printer {} answered {cmd}: {} {}", p.name, print.get("result").cloned().unwrap_or_default(), print.get("reason").cloned().unwrap_or_default());
                        conn.live.write().await.reply = Some(print.clone());
                    }
                    let mut live = conn.live.write().await;
                    if live.print.is_null() {
                        live.print = json!({});
                    }
                    status::merge(&mut live.print, print);
                    live.last_update = Some(now());
                    drop(live);
                    conn.tick.send_modify(|t| *t += 1);
                }
            }
            Ok(_) => {}
            Err(e) => {
                let msg = describe_mqtt_error(&e);
                {
                    let mut live = conn.live.write().await;
                    if live.connected || live.last_error.as_deref() != Some(&msg) {
                        tracing::warn!("printer {} ({}): {msg}", p.name, p.host);
                    }
                    live.connected = false;
                    live.last_error = Some(msg);
                }
                conn.tick.send_modify(|t| *t += 1);
                // Back off while it stays away (a printer that is switched
                // off), but come back at once when a print is waiting for it.
                let max = Duration::from_secs(settings.read().map(|s| s.reconnect_max_secs).unwrap_or(30) as u64).max(FIRST);
                tokio::select! {
                    _ = tokio::time::sleep(backoff.min(max)) => backoff = (backoff * 2).min(max),
                    _ = conn.wake.notified() => backoff = FIRST,
                }
            }
        }
    }
}

fn describe_mqtt_error(e: &rumqttc::ConnectionError) -> String {
    use rumqttc::{ConnectReturnCode, ConnectionError};
    match e {
        ConnectionError::ConnectionRefused(ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized) => {
            "access code rejected — check it on the printer screen (it changes when LAN mode is toggled)".into()
        }
        ConnectionError::Io(io) => format!("cannot reach printer: {io}"),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_command_addresses_the_sd_card() {
        let p1 = build_print_command("Toy_egg.3mf", "Metadata/plate_1.gcode", &[0], &PrintOptions::default(), false);
        assert_eq!(p1["print"]["url"], "file:///sdcard/Toy_egg.3mf");
        assert_eq!((p1["print"]["task_id"].clone(), p1["print"]["project_id"].clone()), (json!("0"), json!("0")));
        assert_eq!(p1["print"]["subtask_name"], "Toy_egg");
        let h2 = build_print_command("Toy_egg.3mf", "Metadata/plate_1.gcode", &[0], &PrintOptions::default(), true);
        assert_eq!(h2["print"]["url"], "ftp:///Toy_egg.3mf");
        assert_eq!(refusal(Some(&json!({"command": "project_file", "result": "FAIL", "reason": "file not found"})), "project_file").as_deref(), Some("file not found"));
        assert_eq!(refusal(Some(&json!({"command": "project_file", "result": "SUCCESS"})), "project_file"), None);
        assert_eq!(refusal(Some(&json!({"command": "ledctrl", "result": "FAIL"})), "project_file"), None);
    }

    #[test]
    fn knows_which_printers_ignore_the_drying_command() {
        assert!(ignores_drying_command("Bambu Lab P1S", "Bambu Lab P1S 0.4 nozzle"));
        assert!(ignores_drying_command("", "Bambu Lab P1P 0.4 nozzle"));
        assert!(!ignores_drying_command("Bambu Lab X1 Carbon", "Bambu Lab X1 Carbon 0.4 nozzle"));
        assert!(!ignores_drying_command("Bambu Lab H2D", "Bambu Lab H2D 0.4 nozzle"));
    }

    #[test]
    fn drying_commands() {
        let d = DrySetting { temp: 70, hours: 8, rotate: true };
        let start = drying_command(128, Some(&d));
        let p = &start["print"];
        assert_eq!(p["command"], "ams_filament_drying");
        assert_eq!((p["ams_id"].clone(), p["temp"].clone(), p["duration"].clone()), (json!(128), json!(70), json!(8)));
        assert_eq!((p["mode"].clone(), p["cooling_temp"].clone(), p["rotate_tray"].clone()), (json!(1), json!(45), json!(true)));
        let stop = drying_command(0, None);
        assert_eq!((stop["print"]["ams_id"].clone(), stop["print"]["mode"].clone()), (json!(0), json!(0)));
        let fail = json!({"command": "ams_filament_drying", "result": "fail", "reason": "power"});
        assert_eq!(refusal(Some(&fail), "ams_filament_drying").as_deref(), Some("power"));
        assert_eq!(refusal(Some(&fail), "project_file"), None);
    }

    #[test]
    fn tray_commands() {
        let pla = TraySetting { kind: "PLA".into(), color: "#ff8800".into(), info_idx: "GFL99".into(), temp_min: 190, temp_max: 240 };
        let p = &tray_setting_command(6, &pla)["print"];
        assert_eq!(p["command"], "ams_filament_setting");
        assert_eq!((p["ams_id"].clone(), p["tray_id"].clone(), p["slot_id"].clone()), (json!(1), json!(2), json!(2)));
        assert_eq!(p["tray_color"], "FF8800FF");
        assert_eq!((p["tray_type"].clone(), p["tray_info_idx"].clone()), (json!("PLA"), json!("GFL99")));
        assert_eq!((p["nozzle_temp_min"].clone(), p["nozzle_temp_max"].clone()), (json!(190), json!(240)));
        let ext = &tray_setting_command(EXTERNAL_TRAY, &pla)["print"];
        assert_eq!((ext["ams_id"].clone(), ext["tray_id"].clone()), (json!(255), json!(254)));
        let ht = &tray_setting_command(129, &pla)["print"];
        assert_eq!((ht["ams_id"].clone(), ht["tray_id"].clone(), ht["slot_id"].clone()), (json!(129), json!(0), json!(0)));

        let r = &reread_command(5).unwrap()["print"];
        assert_eq!((r["command"].clone(), r["ams_id"].clone(), r["slot_id"].clone()), (json!("ams_get_rfid"), json!(1), json!(1)));
        let r = &reread_command(128).unwrap()["print"];
        assert_eq!((r["ams_id"].clone(), r["slot_id"].clone()), (json!(128), json!(0)));
        assert!(reread_command(EXTERNAL_TRAY).is_err());
    }

    #[test]
    fn print_command_mapping() {
        let cmd = build_print_command("benchy.3mf", "Metadata/plate_2.gcode", &[5, -1, 254], &PrintOptions::default(), false);
        let p = &cmd["print"];
        assert_eq!(p["command"], "project_file");
        assert_eq!(p["param"], "Metadata/plate_2.gcode");
        assert_eq!(p["url"], "file:///sdcard/benchy.3mf");
        assert_eq!(p["ams_mapping"], json!([5, -1, -1]));
        assert_eq!(p["ams_mapping2"][0], json!({"ams_id": 1, "slot_id": 1}));
        assert_eq!(p["ams_mapping2"][2], json!({"ams_id": 255, "slot_id": 0}));
        assert_eq!(p["use_ams"], true);
        assert_eq!(p["subtask_name"], "benchy");
    }

    #[test]
    fn external_only_disables_ams() {
        let cmd = build_print_command("a.3mf", "Metadata/plate_1.gcode", &[254], &PrintOptions::default(), false);
        assert_eq!(cmd["print"]["use_ams"], false);
    }
}
