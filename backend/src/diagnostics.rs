//! "Copy diagnostics": what someone helping needs, in one block of text —
//! versions, printers, settings in brief, recent failures and the last log
//! lines, with access codes, tokens and serial numbers taken out.

use crate::state::Shared;
use std::{collections::VecDeque, fmt::Write as _, io, sync::Mutex};

/// Log lines kept for the report.
const LINES: usize = 300;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// A tracing writer that keeps the last log lines in memory.
#[derive(Clone, Copy)]
pub struct LogBuffer;

pub struct LineWriter(Vec<u8>);

impl io::Write for LineWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let mut g = LOG.lock().unwrap_or_else(|e| e.into_inner());
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if g.len() >= LINES {
                g.pop_front();
            }
            g.push_back(line.to_string());
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LineWriter;
    fn make_writer(&'a self) -> Self::Writer {
        LineWriter(Vec::new())
    }
}

/// Everything that must not leave the server, longest first so a secret
/// containing another is replaced whole.
fn secrets(d: &crate::store::Data) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |s: &str, label: &str| {
        let s = s.trim();
        if s.len() >= 4 {
            out.push((s.to_string(), label.to_string()));
        }
    };
    for p in &d.printers {
        add(&p.access_code, "[access code]");
        let tail: String = p.serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        add(&p.serial, &format!("[serial …{tail}]"));
    }
    if let Some(t) = &d.bambu.token {
        add(t, "[token]");
    }
    if let Some(t) = &d.thingiverse_token {
        add(t, "[token]");
    }
    add(&d.notify.token, "[token]");
    for t in &d.notify.targets {
        add(&t.telegram_token, "[token]");
        add(&t.ntfy_token, "[token]");
        add(&t.ntfy_url, "[ntfy topic]");
        add(&t.telegram_chat, "[chat]");
        add(&t.push_endpoint, "[push address]");
        add(&t.push_p256dh, "[key]");
        add(&t.push_auth, "[key]");
    }
    if let Some(k) = &d.webpush_key {
        add(k, "[key]");
    }
    out.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
    out
}

pub fn redact(text: &str, secrets: &[(String, String)]) -> String {
    let mut t = text.to_string();
    for (s, label) in secrets {
        t = t.replace(s.as_str(), label);
    }
    t
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

pub async fn report(state: &Shared) -> String {
    let data = state.store.read(|d| d.clone()).await;
    let mut r = String::new();
    let in_addon = std::env::var("SUPERVISOR_TOKEN").is_ok_and(|t| !t.is_empty());
    let _ = writeln!(r, "AirSlice {} ({} {}), {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, if in_addon { "Home Assistant add-on" } else { "standalone" });
    if !state.cfg.base_path.is_empty() {
        let _ = writeln!(r, "Base path: {}", state.cfg.base_path);
    }
    let profiles = state.slicer.profiles().await;
    let _ = writeln!(
        r,
        "OrcaSlicer: {}{}",
        if state.slicer.available() { state.slicer.version().await.unwrap_or_else(|| "found".into()) } else { "not found".into() },
        match &profiles {
            Ok(_) => String::new(),
            Err(e) => format!(", profiles: {e:#}"),
        }
    );
    let _ = writeln!(r, "MakerWorld account: {}, Thingiverse token: {}", yes(data.bambu.token.is_some()), yes(data.thingiverse_token.is_some()));

    let _ = writeln!(r, "\nPrinters ({}):", data.printers.len());
    for p in &data.printers {
        let s = state.printers.status(&p.id).await;
        let _ = writeln!(r, "- {}: {} / {}, FTP {:?}, camera {}, plug {}", p.name, p.printer_model, p.machine_profile, p.ftp_data_mode, if p.camera.is_empty() { "none" } else { &p.camera }, yes(p.power_switch.is_some()));
        let _ = writeln!(
            r,
            "  {}{}, state {}, AMS units {}, nozzle {} {}, wifi {}",
            if s.connected { "connected" } else { "not connected" },
            s.last_error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default(),
            s.state.as_deref().unwrap_or("?"),
            s.ams_units.len(),
            s.nozzle_diameter.as_deref().unwrap_or("?"),
            s.nozzle_type.as_deref().unwrap_or(""),
            s.wifi_dbm.map(|w| format!("{w} dBm")).unwrap_or_else(|| "?".into()),
        );
        if !s.hms.is_empty() {
            let _ = writeln!(r, "  HMS: {}", s.hms.join(", "));
        }
    }

    let n = &data.notify;
    let channels: Vec<String> = n.targets.iter().map(|t| format!("{}{}", t.channel, if t.enabled { "" } else { " (off)" })).collect();
    let _ = writeln!(r, "\nNotifications: {}, {}", if n.enabled { "on" } else { "off" }, if channels.is_empty() { "none".into() } else { channels.join(", ") });
    let _ = writeln!(r, "Spoolman: {}", if data.spoolman.url.is_empty() { "off" } else { "set up" });
    let _ = writeln!(r, "Queue: {} prints", data.queue.len());

    let jobs = state.store.read(|d| d.jobs.clone()).await;
    let failed: Vec<_> = jobs.iter().filter(|j| j.error.is_some()).take(5).collect();
    let _ = writeln!(r, "\nJobs: {} kept, last failures:", jobs.len());
    for j in &failed {
        let ago = crate::jobs::fmt_duration((crate::models::now() - j.updated_at).max(0) as u64);
        let _ = writeln!(r, "- {} ({:?}, {ago} ago): {}", j.model_title, j.source, j.error.as_deref().unwrap_or(""));
    }
    if failed.is_empty() {
        let _ = writeln!(r, "- none");
    }

    let _ = writeln!(r, "\nRecent log:");
    for line in LOG.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        let _ = writeln!(r, "{line}");
    }
    redact(&r, &secrets(&data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_taken_out() {
        let mut d = crate::store::Data::default();
        let mut p: crate::models::Printer = serde_json::from_value(serde_json::json!({
            "id": "p", "name": "Workshop", "host": "192.168.1.20", "serial": "01P00A123456789", "access_code": "12345678",
            "printer_model": "Bambu Lab P1S", "machine_profile": "Bambu Lab P1S 0.4 nozzle"
        }))
        .unwrap();
        p.access_code = "12345678".into();
        d.printers.push(p);
        d.bambu.token = Some("eyJabc.secret.token".into());
        let text = "connecting to 01P00A123456789 with 12345678, token eyJabc.secret.token";
        assert_eq!(redact(text, &secrets(&d)), "connecting to [serial …6789] with [access code], token [token]");
    }

    #[test]
    fn keeps_the_last_lines() {
        for i in 0..(LINES + 5) {
            drop(LineWriter(format!("line {i}\n").into_bytes()));
        }
        let g = LOG.lock().unwrap();
        assert_eq!(g.len(), LINES);
        assert_eq!(g.back().map(String::as_str), Some(format!("line {}", LINES + 4).as_str()));
    }
}
