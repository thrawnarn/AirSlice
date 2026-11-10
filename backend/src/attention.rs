//! Noticing when a printer needs a person: a print that paused, a serious
//! printer message, or contact lost during a print. A paused print isn't a
//! failed job, so nothing else reports it.

use crate::{
    models::{Job, JobState, now},
    printer::status::{Problem, StatusView, hms_explained},
    state::Shared,
};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// A pause or message has to last this long. The printer can flip between
/// paused and running within seconds while someone is fixing it, and the
/// reason may arrive in the report after the pause.
const HOLD_SECS: i64 = 10;
/// Long enough to ride out a Wi-Fi drop.
const LOST_AFTER_SECS: i64 = 300;
/// The same reason isn't repeated within this time, as trying again at the
/// printer can bring the same message straight back.
const REPEAT_AFTER_SECS: i64 = 900;
/// A pause without a message counts as this code for `REPEAT_AFTER_SECS`.
const PLAIN_PAUSE: &str = "pause";
/// How long after our own pause command a pause is taken to be that one.
const OWN_PAUSE_SECS: i64 = 60;

#[derive(Debug, PartialEq)]
pub enum Alert {
    /// The print paused; the printer's active messages say why.
    Paused(Vec<Problem>),
    /// New serious or fatal messages, without a pause to report.
    Trouble(Vec<Problem>),
    Lost,
    Back,
}

#[derive(Default)]
struct Watch {
    pause_since: Option<i64>,
    pause_handled: bool,
    codes_since: HashMap<String, i64>,
    /// When each reason was last sent.
    told: HashMap<String, i64>,
    lost_since: Option<i64>,
    lost_told: bool,
}

fn own_pauses() -> &'static Mutex<HashMap<String, i64>> {
    static OWN: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    OWN.get_or_init(Default::default)
}

/// The person pressed pause in AirSlice: they don't need telling.
pub fn paused_by_us(printer_id: &str) {
    own_pauses().lock().unwrap().insert(printer_id.to_string(), now());
}

fn is_own_pause(printer_id: &str, at: i64) -> bool {
    own_pauses().lock().unwrap().get(printer_id).is_some_and(|t| at - *t < OWN_PAUSE_SECS)
}

/// What one look at a printer calls for. `printing` = a started job is on it.
fn step(w: &mut Watch, s: &StatusView, printing: bool, own_pause: bool, at: i64) -> Vec<Alert> {
    let mut out = Vec::new();
    if !s.connected {
        // The last report is stale; start over when the printer is back.
        w.pause_since = None;
        w.pause_handled = false;
        w.codes_since.clear();
        if !printing {
            w.lost_since = None;
            w.lost_told = false;
            return out;
        }
        let since = *w.lost_since.get_or_insert(at);
        if !w.lost_told && at - since >= LOST_AFTER_SECS {
            w.lost_told = true;
            out.push(Alert::Lost);
        }
        return out;
    }
    w.lost_since = None;
    if std::mem::take(&mut w.lost_told) {
        out.push(Alert::Back);
    }
    w.told.retain(|_, sent| at - *sent < REPEAT_AFTER_SECS);

    if s.state.as_deref() == Some("PAUSE") {
        let since = *w.pause_since.get_or_insert(at);
        if !w.pause_handled && at - since >= HOLD_SECS {
            w.pause_handled = true;
            let codes: Vec<&str> = if s.problems.is_empty() { vec![PLAIN_PAUSE] } else { s.problems.iter().map(|p| p.code.as_str()).collect() };
            if !own_pause && codes.iter().any(|c| !w.told.contains_key(*c)) {
                w.told.extend(codes.iter().map(|c| (c.to_string(), at)));
                out.push(Alert::Paused(s.problems.clone()));
            }
        }
    } else {
        w.pause_since = None;
        w.pause_handled = false;
    }

    w.codes_since.retain(|c, _| s.hms.contains(c));
    let due: Vec<Problem> = s
        .problems
        .iter()
        // A code with its own explanation is about a print that didn't
        // start, which the job's failure already reports.
        .filter(|p| matches!(p.severity, "serious" | "fatal") && hms_explained(&p.code).is_none())
        .filter(|p| {
            let since = *w.codes_since.entry(p.code.clone()).or_insert(at);
            at - since >= HOLD_SECS && !w.told.contains_key(&p.code)
        })
        .cloned()
        .collect();
    if !due.is_empty() {
        w.told.extend(due.iter().map(|p| (p.code.clone(), at)));
        out.push(Alert::Trouble(due));
    }
    out
}

/// Look at every printer for the life of the server.
pub async fn watch(state: Shared) {
    let mut watches: HashMap<String, Watch> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (printers, started) = state
            .store
            .read(|d| (d.printers.clone(), d.jobs.iter().filter(|j| j.state == JobState::Started && j.print.is_some()).cloned().collect::<Vec<Job>>()))
            .await;
        watches.retain(|id, _| printers.iter().any(|p| p.id == *id));
        let at = now();
        for printer in &printers {
            let job = started.iter().find(|j| j.print.as_ref().is_some_and(|p| p.printer_id == printer.id));
            let status = state.printers.status(&printer.id).await;
            let w = watches.entry(printer.id.clone()).or_default();
            for alert in step(w, &status, job.is_some(), is_own_pause(&printer.id, at), at) {
                crate::events::attention(&state, printer, job, &status, alert).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str, codes: &[&str]) -> StatusView {
        StatusView {
            connected: true,
            state: Some(state.into()),
            hms: codes.iter().map(|c| c.to_string()).collect(),
            problems: codes.iter().map(|c| Problem::new(c)).collect(),
            ..Default::default()
        }
    }

    /// `(start, end)` in seconds, looking every 5 s as `watch` does.
    fn run(w: &mut Watch, s: &StatusView, printing: bool, from: i64, to: i64) -> Vec<Alert> {
        (from..to).step_by(5).flat_map(|at| step(w, s, printing, false, at)).collect()
    }

    const OUT_OF_FILAMENT: &str = "1800_2000_0002_0001";
    const PTFE: &str = "1800_2000_0002_0006";

    /// The evening of 2026-11-09 on a real P1S: filament ran out, then the
    /// PTFE alarm came and went while someone tried to resume.
    #[test]
    fn a_real_filament_runout_notifies_once_per_reason() {
        let mut w = Watch::default();
        // A "common" message without a pause: the printer is purging.
        assert!(run(&mut w, &status("RUNNING", &["1800_2000_0003_0001"]), true, 0, 20).is_empty());
        let paused = run(&mut w, &status("PAUSE", &[OUT_OF_FILAMENT]), true, 25, 300);
        assert_eq!(paused.len(), 1, "{paused:?}");
        assert!(matches!(&paused[0], Alert::Paused(p) if p[0].code == OUT_OF_FILAMENT));

        assert!(run(&mut w, &status("RUNNING", &[]), true, 4400, 4410).is_empty());
        let ptfe = run(&mut w, &status("PAUSE", &[PTFE]), true, 4415, 4600);
        assert!(matches!(&ptfe[..], [Alert::Paused(p)] if p[0].code == PTFE), "{ptfe:?}");

        // Resumed; the same alarm comes back at once.
        assert!(run(&mut w, &status("RUNNING", &[]), true, 4605, 4620).is_empty());
        assert!(run(&mut w, &status("PAUSE", &[PTFE]), true, 4625, 4640).is_empty());
    }

    #[test]
    fn a_short_pause_says_nothing() {
        let mut w = Watch::default();
        assert!(run(&mut w, &status("PAUSE", &[PTFE]), true, 0, 5).is_empty());
        assert!(run(&mut w, &status("RUNNING", &[]), true, 5, 30).is_empty());
    }

    #[test]
    fn a_pause_without_a_message_still_notifies() {
        let mut w = Watch::default();
        assert_eq!(run(&mut w, &status("PAUSE", &[]), true, 0, 60), vec![Alert::Paused(Vec::new())]);
    }

    #[test]
    fn our_own_pause_is_not_reported() {
        let mut w = Watch::default();
        let s = status("PAUSE", &[]);
        for at in (0..60).step_by(5) {
            assert!(step(&mut w, &s, true, at < 30, at).is_empty());
        }
    }

    #[test]
    fn a_serious_message_counts_without_a_print_but_a_common_one_does_not() {
        let mut w = Watch::default();
        assert!(run(&mut w, &status("IDLE", &["0300_0100_0003_0001"]), false, 0, 60).is_empty());
        let out = run(&mut w, &status("IDLE", &["0300_0100_0001_0001"]), false, 65, 200);
        assert!(matches!(&out[..], [Alert::Trouble(p)] if p.len() == 1), "{out:?}");
    }

    #[test]
    fn the_start_refusal_is_left_to_the_job() {
        let mut w = Watch::default();
        assert!(run(&mut w, &status("IDLE", &["0500_0500_0001_0007"]), false, 0, 60).is_empty());
    }

    #[test]
    fn losing_contact_counts_only_during_a_print() {
        let off = StatusView::default();
        let mut w = Watch::default();
        assert!(run(&mut w, &off, false, 0, 1000).is_empty(), "a plug turned off when idle");
        assert!(run(&mut w, &off, true, 1000, 1200).is_empty(), "a short drop");
        assert!(step(&mut w, &status("RUNNING", &[]), true, false, 1205).is_empty());
        assert_eq!(run(&mut w, &off, true, 1210, 2000), vec![Alert::Lost]);
        assert_eq!(step(&mut w, &status("RUNNING", &[]), true, false, 2005), vec![Alert::Back]);
    }
}
