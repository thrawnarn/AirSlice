//! A printer's power plug: a Home Assistant switch. AirSlice turns it on to
//! send a print, can turn it off once a print is done and the nozzle has
//! cooled, and reads the plug's power and energy sensors (found on the
//! switch's device). Optional; without Home Assistant none of this runs.

use crate::{
    models::{JobState, Printer, now},
    state::Shared,
};
use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Mutex, time::Duration};

/// What the printer card shows about the plug.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PowerView {
    /// `None` when Home Assistant doesn't answer.
    pub on: Option<bool>,
    pub watts: Option<f64>,
    /// Seconds until auto-off turns it off, if counting down.
    pub off_in: Option<i64>,
    /// Seconds since the plug was seen going on, while the printer hasn't
    /// answered yet (elapsed, so the phone's clock doesn't matter).
    pub on_secs: Option<i64>,
}

struct Seen {
    at: i64,
    view: PowerView,
}

static CACHE: Mutex<Option<HashMap<String, Seen>>> = Mutex::new(None);
/// When each plug went on, as far as AirSlice saw.
static ON_SINCE: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

fn mark_on(id: &str, on: bool) {
    let mut g = ON_SINCE.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if on {
        m.entry(id.to_string()).or_insert_with(now);
    } else {
        m.remove(id);
    }
}
/// Printers armed for auto-off by a print that ended, with since when they
/// have been idle and cool (`None` while still busy or hot).
static IDLE: Mutex<Option<HashMap<String, Option<i64>>>> = Mutex::new(None);

/// A print ended: count down to turning the plug off once it's idle and cool.
pub fn arm(printer_id: &str) {
    IDLE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(printer_id.to_string(), None);
}

async fn ha(state: &Shared) -> anyhow::Result<(String, String)> {
    let s = state.store.read(|d| d.notify.clone()).await;
    crate::notify::endpoint(&s)
}

async fn entity_state(state: &Shared, entity: &str) -> anyhow::Result<Value> {
    if !["switch", "sensor"].iter().any(|d| crate::notify::is_entity_id(entity, d)) {
        bail!("{entity:?} isn't a switch or sensor");
    }
    let (base, token) = ha(state).await?;
    let v: Value = state
        .http
        .get(format!("{base}/states/{entity}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("couldn't reach Home Assistant")?
        .error_for_status()
        .with_context(|| format!("Home Assistant has no {entity}"))?
        .json()
        .await?;
    Ok(v)
}

fn number(v: &Value) -> Option<f64> {
    v.get("state")?.as_str()?.parse().ok()
}

/// Turn the plug on or off.
pub async fn set(state: &Shared, printer: &Printer, on: bool) -> anyhow::Result<()> {
    let Some(switch) = printer.power_switch.as_deref().filter(|s| crate::notify::is_entity_id(s, "switch")) else { bail!("{} has no power plug set", printer.name) };
    let (base, token) = ha(state).await?;
    let service = if on { "turn_on" } else { "turn_off" };
    state
        .http
        .post(format!("{base}/services/switch/{service}"))
        .bearer_auth(token)
        .json(&json!({ "entity_id": switch }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("couldn't reach Home Assistant")?
        .error_for_status()
        .context("Home Assistant refused to switch the plug")?;
    // Turning on again (already on) keeps the earlier time.
    mark_on(&printer.id, on);
    forget(&printer.id);
    if on {
        // The printer reconnects sooner when told to look.
        state.printers.wake(&printer.id).await;
    }
    Ok(())
}

fn forget(id: &str) {
    if let Some(m) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(id);
    }
}

/// The plug's state and power, read at most every 10 s.
pub async fn view(state: &Shared, printer: &Printer) -> Option<PowerView> {
    let switch = printer.power_switch.as_deref()?;
    if let Some(s) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&printer.id))
        && now() - s.at < 10
    {
        let mut v = s.view.clone();
        timers(&mut v, printer);
        return Some(v);
    }
    let mut v = PowerView::default();
    let before = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&printer.id)).and_then(|s| s.view.on);
    if let Ok(s) = entity_state(state, switch).await {
        v.on = match s.get("state").and_then(|s| s.as_str()) {
            Some("on") => Some(true),
            Some("off") => Some(false),
            _ => None,
        };
    }
    // Switched in Home Assistant (or by hand): off → on starts the clock.
    match (before, v.on) {
        (Some(false), Some(true)) => mark_on(&printer.id, true),
        (_, Some(false)) => mark_on(&printer.id, false),
        _ => {}
    }
    // Once the printer answers it has started; nothing to wait for.
    if state.printers.status(&printer.id).await.connected
        && let Some(m) = ON_SINCE.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
    {
        m.remove(&printer.id);
    }
    if let Some(p) = printer.power_sensor.as_deref()
        && let Ok(s) = entity_state(state, p).await
    {
        v.watts = number(&s);
    }
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(printer.id.clone(), Seen { at: now(), view: v.clone() });
    timers(&mut v, printer);
    Some(v)
}

/// The start-up and auto-off clocks, as seconds from now (fresh on every
/// call, and independent of the viewer's clock).
fn timers(v: &mut PowerView, printer: &Printer) {
    let on = v.on == Some(true);
    // A disabled printer never connects: nothing to count down to.
    v.on_secs = ON_SINCE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&printer.id).copied())
        .filter(|_| on && printer.enabled)
        .map(|since| (now() - since).max(0));
    let after = printer.power_off_after_min as i64 * 60;
    v.off_in = IDLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&printer.id).copied().flatten())
        .filter(|_| after > 0 && on)
        .map(|since| (since + after - now()).max(0));
}

/// The energy meter's reading in kWh, to count what a print used.
pub async fn energy_kwh(state: &Shared, printer: &Printer) -> Option<f64> {
    let e = printer.energy_sensor.as_deref()?;
    let s = entity_state(state, e).await.ok()?;
    let value = number(&s)?;
    // Some meters report Wh.
    match s.pointer("/attributes/unit_of_measurement").and_then(|u| u.as_str()) {
        Some("Wh") => Some(value / 1000.0),
        Some("MWh") => Some(value * 1000.0),
        _ => Some(value),
    }
}

/// The power and energy sensors on the same device as the switch.
pub async fn find_sensors(state: &Shared, switch: &str) -> (Option<String>, Option<String>) {
    // It goes into the template as text.
    if !crate::notify::is_entity_id(switch, "switch") {
        return (None, None);
    }
    let Ok((base, token)) = ha(state).await else { return (None, None) };
    let template = format!(
        "{{% set ns = namespace(p='', e='') %}}{{% for e in device_entities(device_id('{switch}')) | select('match', 'sensor\\.') %}}\
         {{% set dc = state_attr(e, 'device_class') %}}{{% if dc == 'power' and not ns.p %}}{{% set ns.p = e %}}{{% endif %}}\
         {{% if dc == 'energy' and not ns.e %}}{{% set ns.e = e %}}{{% endif %}}{{% endfor %}}{{{{ ns.p }}}}|{{{{ ns.e }}}}"
    );
    let Ok(resp) = state.http.post(format!("{base}/template")).bearer_auth(token).json(&json!({ "template": template })).send().await else {
        return (None, None);
    };
    let text = resp.text().await.unwrap_or_default();
    let mut parts = text.trim().splitn(2, '|').map(|s| s.trim().to_string()).map(|s| Some(s).filter(|s| crate::notify::is_entity_id(s, "sensor")));
    (parts.next().flatten(), parts.next().flatten())
}

/// Every minute: turn plugs off for printers whose print ended, that have
/// nothing queued or waiting, and have been idle with a cool nozzle for the
/// printer's `power_off_after_min`. Turning the plug on by hand doesn't arm it.
pub async fn watch(state: Shared) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let printers = state.store.read(|d| d.printers.clone()).await;
        for p in printers.iter().filter(|p| p.power_switch.is_some() && p.power_off_after_min > 0) {
            let s = state.printers.status(&p.id).await;
            let pending = state
                .store
                .read(|d| {
                    d.queue.iter().any(|q| q.printer_id == p.id)
                        || d.jobs.iter().any(|j| {
                            matches!(j.state, JobState::Waiting | JobState::Uploading | JobState::Starting)
                                && (j.slice.as_ref().is_some_and(|s| s.printer_id == p.id) || j.print.as_ref().is_some_and(|pr| pr.printer_id == p.id))
                        })
                })
                .await;
            let resting = s.connected && !s.is_busy() && !pending && s.nozzle_temp.is_some_and(|t| t < 50.0);
            let since = {
                let mut g = IDLE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(entry) = g.get_or_insert_with(HashMap::new).get_mut(&p.id) else { continue };
                if s.is_busy() {
                    // Another print started: it arms again when it ends.
                    g.as_mut().unwrap().remove(&p.id);
                    continue;
                }
                *entry = if resting { Some(entry.unwrap_or_else(now)) } else { None };
                *entry
            };
            let Some(since) = since else { continue };
            if now() - since < p.power_off_after_min as i64 * 60 {
                continue;
            }
            if view(&state, p).await.and_then(|v| v.on) != Some(true) {
                continue;
            }
            match set(&state, p, false).await {
                Ok(()) => tracing::info!("turned off {}'s plug after {} min idle", p.name, p.power_off_after_min),
                Err(e) => tracing::warn!("couldn't turn off {}'s plug: {e:#}", p.name),
            }
            if let Some(m) = IDLE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                m.remove(&p.id);
            }
        }
    }
}
