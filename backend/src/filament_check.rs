//! Will the spools last for this print? A tray that runs short isn't a
//! problem when another tray holds the same filament: with the printer's AMS
//! filament backup on, it carries on from that one.

use crate::{
    models::Job,
    printer::{
        mapping,
        status::{EXTERNAL_TRAY, Tray},
    },
    spoolman::{self, Spool},
    state::Shared,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// A tray holding less than the print needs from it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Short {
    pub tray: i32,
    pub left_g: f64,
    pub need_g: f64,
    /// Trays with the same filament that would take over.
    pub backups: Vec<i32>,
    /// The backups hold enough and the printer will use them: nothing to do.
    pub covered: bool,
    /// The printer's filament backup is off, so the backups won't be used.
    pub backup_off: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Check {
    pub short: Vec<Short>,
    /// Trays the print uses whose remaining amount nothing reports.
    pub unknown: Vec<i32>,
}

/// What the AMS itself would switch between. Colour and material aren't
/// enough when both trays know their filament id: PLA Basic and PLA Matte in
/// the same white are different filaments to the printer.
fn same_filament(a: &Tray, b: &Tray) -> bool {
    let ids_differ = matches!((&a.info_idx, &b.info_idx), (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() && x != y);
    !ids_differ && a.kind.eq_ignore_ascii_case(&b.kind) && !a.color.is_empty() && a.color.eq_ignore_ascii_case(&b.color)
}

/// Spoolman's figure, else the AMS's estimate (percent of the spool).
fn left_g(t: &Tray, spool: Option<&Spool>) -> Option<f64> {
    spool.and_then(|s| s.remaining_g).or_else(|| t.remain.map(|r| r as f64 / 100.0 * t.spool_g.unwrap_or(1000) as f64))
}

pub fn check(use_g: &[(i32, f64)], trays: &[&Tray], spools: &HashMap<i32, Spool>, backup_on: Option<bool>) -> Check {
    let mut need: BTreeMap<i32, f64> = BTreeMap::new();
    for (tray, g) in use_g {
        *need.entry(*tray).or_default() += g;
    }
    // What each AMS tray has beyond its own share; `None` = nobody knows.
    let mut spare: HashMap<i32, Option<f64>> = trays
        .iter()
        .filter(|t| t.id != EXTERNAL_TRAY)
        .map(|t| (t.id, left_g(t, spools.get(&t.id)).map(|l| (l - need.get(&t.id).copied().unwrap_or(0.0)).max(0.0))))
        .collect();

    let mut out = Check::default();
    for (&tray, &need_g) in &need {
        let Some(t) = trays.iter().find(|t| t.id == tray) else { continue };
        let Some(left) = left_g(t, spools.get(&tray)) else {
            out.unknown.push(tray);
            continue;
        };
        if left >= need_g {
            continue;
        }
        let mut missing = need_g - left;
        let mut backups = Vec::new();
        if tray != EXTERNAL_TRAY {
            for other in trays.iter().filter(|o| o.id != tray && o.id != EXTERNAL_TRAY && same_filament(t, o)) {
                match spare.get_mut(&other.id) {
                    Some(Some(s)) if *s <= 0.0 => continue,
                    Some(Some(s)) => {
                        let taken = s.min(missing);
                        *s -= taken;
                        missing -= taken;
                    }
                    // A spool of unknown size: give it the benefit of the doubt.
                    _ => missing = 0.0,
                }
                backups.push(other.id);
                if missing <= 0.0 {
                    break;
                }
            }
        }
        let backup_off = !backups.is_empty() && backup_on == Some(false);
        let covered = missing <= 0.0 && !backup_off;
        out.short.push(Short { tray, left_g: left, need_g, backups, covered, backup_off });
    }
    out
}

/// Check a sliced job on a printer. `chosen` = the trays picked for it; else
/// the ones its queue entry has, else what the printer would match by itself.
pub async fn for_job(state: &Shared, job: &Job, printer_id: &str, chosen: Option<Vec<i32>>) -> Check {
    let Some(slice) = &job.slice else { return Check::default() };
    let status = state.printers.status(printer_id).await;
    let queued = state.store.read(|d| d.queue.iter().find(|q| q.job_id == job.id).and_then(|q| q.options.ams_mapping.clone())).await;
    let trays = chosen.or(queued).unwrap_or_else(|| mapping::suggest_with(&slice.filaments, slice.filament_slots, &status, slice.preferred_trays.as_deref().unwrap_or(&[])).ams_mapping);
    let use_g = spoolman::use_per_tray(&slice.filaments, &trays, 1.0);
    let spools = spoolman::for_printer(state, printer_id).await;
    check(&use_g, &status.usable_trays(), &spools, status.ams_backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray(id: i32, kind: &str, color: &str, idx: &str, remain: Option<i32>) -> Tray {
        Tray { id, ams: Some(0), slot: Some(id as u32), kind: kind.into(), color: color.into(), info_idx: Some(idx.into()), remain, spool_g: Some(1000), ..Default::default() }
    }

    fn run(use_g: &[(i32, f64)], trays: &[Tray], backup: Option<bool>) -> Check {
        check(use_g, &trays.iter().collect::<Vec<_>>(), &HashMap::new(), backup)
    }

    const WHITE: &str = "#FFFFFF";

    #[test]
    fn enough_says_nothing() {
        assert_eq!(run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(50))], Some(true)), Check::default());
    }

    #[test]
    fn short_without_a_spare_warns() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", "#000000", "GFA01", Some(90))], Some(true));
        assert_eq!(c.short, vec![Short { tray: 0, left_g: 40.0, need_g: 90.0, backups: vec![], covered: false, backup_off: false }]);
    }

    #[test]
    fn the_same_filament_in_another_tray_covers_it() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "pla", "#ffffff", "GFA01", Some(90))], Some(true));
        assert!(c.short[0].covered);
        assert_eq!(c.short[0].backups, vec![1]);
    }

    #[test]
    fn another_kind_of_the_same_white_is_not_a_spare() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", WHITE, "GFA00", Some(90))], Some(true));
        assert!(!c.short[0].covered);
        assert!(c.short[0].backups.is_empty());
    }

    #[test]
    fn a_spare_that_is_too_small_still_warns() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", WHITE, "GFA01", Some(2))], Some(true));
        assert!(!c.short[0].covered);
        assert_eq!(c.short[0].backups, vec![1]);
    }

    #[test]
    fn the_spare_print_share_is_not_counted_twice() {
        // Both trays are used by this print; the second has nothing to give.
        let c = run(&[(0, 90.0), (1, 880.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", WHITE, "GFA01", Some(90))], Some(true));
        assert_eq!(c.short.len(), 1);
        assert!(!c.short[0].covered, "{c:?}");
    }

    #[test]
    fn one_spare_cant_save_two_trays() {
        let trays = [tray(0, "PLA", WHITE, "GFA01", Some(0)), tray(1, "PLA", WHITE, "GFA01", Some(0)), tray(2, "PLA", WHITE, "GFA01", Some(10))];
        let c = run(&[(0, 90.0), (1, 90.0)], &trays, Some(true));
        assert!(c.short[0].covered);
        assert!(!c.short[1].covered, "{c:?}");
    }

    #[test]
    fn backup_turned_off_on_the_printer_warns() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", WHITE, "GFA01", Some(90))], Some(false));
        assert!(!c.short[0].covered && c.short[0].backup_off);
    }

    #[test]
    fn the_external_spool_has_no_backup() {
        let c = run(&[(EXTERNAL_TRAY, 90.0)], &[tray(EXTERNAL_TRAY, "PLA", WHITE, "GFA01", Some(4)), tray(1, "PLA", WHITE, "GFA01", Some(90))], Some(true));
        assert!(!c.short[0].covered);
    }

    #[test]
    fn unknown_amounts_are_named_not_guessed() {
        let c = run(&[(0, 90.0)], &[tray(0, "PLA", WHITE, "GFA01", None)], Some(true));
        assert_eq!(c, Check { short: vec![], unknown: vec![0] });
    }

    #[test]
    fn spoolman_wins_over_the_ams() {
        let t = tray(0, "PLA", WHITE, "GFA01", Some(90));
        let spool = Spool { id: 1, name: "x".into(), material: "PLA".into(), color: WHITE.into(), remaining_g: Some(30.0), price_per_g: None, tag: None };
        let c = check(&[(0, 90.0)], &[&t], &HashMap::from([(0, spool)]), Some(true));
        assert_eq!(c.short[0].left_g, 30.0);
    }
}
