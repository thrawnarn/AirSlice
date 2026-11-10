//! What just happened, for clients that poll (`GET api/events`): Home Assistant
//! turns these into events for automations. Kept in memory, whatever the
//! notification settings say.

use crate::models::now;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const KEEP: usize = 200;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub at: i64,
    /// `print_started`, `first_layer`, `almost_done`, `print_finished`, `print_failed`,
    /// `print_stopped`, `job_failed`, `paused`, `printer_problem`, `filament_low`,
    /// `contact_lost`, `contact_back` or `queue_ready`.
    pub kind: &'static str,
    pub printer_id: String,
    pub job_id: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
}

static LOG: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());
// Counted from the start time, so a client's position stays valid over a restart.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn next_seq() -> u64 {
    let _ = SEQ.compare_exchange(0, now() as u64 * 1000, Ordering::SeqCst, Ordering::SeqCst);
    SEQ.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn record(kind: &'static str, printer_id: &str, job_id: Option<&str>, title: Option<String>, message: Option<String>) {
    let entry = Entry { seq: next_seq(), at: now(), kind, printer_id: printer_id.to_string(), job_id: job_id.map(String::from), title, message };
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(entry);
    while log.len() > KEEP {
        log.pop_front();
    }
}

/// The position to continue from, and what came after `after`. Without
/// `after` nothing is returned: a new client shouldn't replay the past.
pub fn since(after: Option<u64>) -> (u64, Vec<Entry>) {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let latest = log.back().map(|e| e.seq).unwrap_or_else(|| after.unwrap_or(0));
    let events = after.map(|a| log.iter().filter(|e| e.seq > a).cloned().collect()).unwrap_or_default();
    (latest, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_gets_each_event_once() {
        let (start, none) = since(None);
        assert!(none.is_empty());
        record("paused", "unique-printer", None, Some("Vase".into()), None);
        let (latest, new) = since(Some(start));
        assert!(new.iter().any(|e| e.kind == "paused" && e.printer_id == "unique-printer"));
        assert!(since(Some(latest)).1.iter().all(|e| e.printer_id != "unique-printer"));
    }
}
