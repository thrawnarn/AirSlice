//! Matching the filaments a sliced plate needs to what is loaded on the printer.

use super::status::{EXTERNAL_TRAY, StatusView, Tray};
use crate::models::SlicedFilament;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MappingSuggestion {
    /// One entry per project filament slot (index = slot - 1).
    pub ams_mapping: Vec<i32>,
    pub use_ams: bool,
    /// Human-readable warnings, e.g. type mismatch or nothing loaded.
    pub warnings: Vec<String>,
    /// True when every used filament found a tray of the same type.
    pub complete: bool,
}

fn rgb(hex: &str) -> Option<(i32, i32, i32)> {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let p = |i: usize| i32::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

/// Rough perceptual distance between two colors (weighted RGB).
pub fn color_distance(a: &str, b: &str) -> i32 {
    match (rgb(a), rgb(b)) {
        (Some((r1, g1, b1)), Some((r2, g2, b2))) => {
            let (dr, dg, db) = (r1 - r2, g1 - g2, b1 - b2);
            2 * dr * dr + 4 * dg * dg + 3 * db * db
        }
        _ => i32::MAX / 2,
    }
}

/// "PLA Basic", "PLA-CF", "PLA+" -> "PLA"; used as a fallback match.
fn base_type(t: &str) -> String {
    t.trim()
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("")
        .trim_end_matches("CF")
        .to_string()
}

fn same_type(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Pick a tray for each used filament.
///
/// Preference: exact type match with the closest color, then same base
/// material (e.g. "PLA" for "PLA Silk"), each tray used at most once while
/// unused trays remain. With no AMS, everything goes to the external spool.
#[cfg(test)]
pub fn suggest(filaments: &[SlicedFilament], slots: u32, status: &StatusView) -> MappingSuggestion {
    suggest_with(filaments, slots, status, &[])
}

/// Like `suggest`, but slots in `pinned` (index = slot - 1, value = tray)
/// keep their tray when it is still loaded with the same material, e.g. the
/// tray the user picked before slicing.
pub fn suggest_with(filaments: &[SlicedFilament], slots: u32, status: &StatusView, pinned: &[i32]) -> MappingSuggestion {
    let slots = slots.max(filaments.iter().map(|f| f.id).max().unwrap_or(0)) as usize;
    let mut mapping = vec![-1; slots];
    let mut warnings = Vec::new();
    let mut complete = true;
    let trays: Vec<&Tray> = status.usable_trays();
    let mut taken: Vec<i32> = Vec::new();
    let mut remaining: Vec<&SlicedFilament> = Vec::new();
    for f in filaments {
        let want = pinned.get(f.id as usize - 1).copied().unwrap_or(-1);
        match trays.iter().find(|t| t.id == want && base_type(&t.kind) == base_type(&f.kind)) {
            Some(t) if want >= 0 => {
                mapping[f.id as usize - 1] = t.id;
                taken.push(t.id);
            }
            _ => remaining.push(f),
        }
    }
    if remaining.is_empty() && !filaments.is_empty() {
        return finish(mapping, warnings, complete);
    }
    let filaments: Vec<SlicedFilament> = remaining.into_iter().cloned().collect();
    let filaments = filaments.as_slice();

    if trays.is_empty() {
        // Nothing reported (no AMS, or status not received yet): use the
        // external spool, which is what the printer does on its own.
        for f in filaments {
            if let Some(m) = mapping.get_mut(f.id as usize - 1) {
                *m = EXTERNAL_TRAY;
            }
        }
        if !filaments.is_empty() {
            warnings.push("No filament info from the printer — using the external spool.".into());
        }
        if filaments.len() > 1 {
            warnings.push("This plate uses several filaments but no AMS was found.".into());
            complete = false;
        }
        return finish(mapping, warnings, complete);
    }

    // Place the filaments with the fewest candidate trays first.
    let mut order: Vec<&SlicedFilament> = filaments.iter().collect();
    order.sort_by_key(|f| trays.iter().filter(|t| same_type(&t.kind, &f.kind)).count());

    for f in order {
        let pick = |pred: &dyn Fn(&Tray) -> bool, allow_taken: bool| -> Option<&Tray> {
            trays
                .iter()
                .copied()
                .filter(|t| pred(t) && (allow_taken || !taken.contains(&t.id)))
                .min_by_key(|t| color_distance(&t.color, &f.color))
        };
        let exact = |t: &Tray| same_type(&t.kind, &f.kind);
        let base = |t: &Tray| base_type(&t.kind) == base_type(&f.kind) && !base_type(&f.kind).is_empty();

        let chosen = pick(&exact, false)
            .or_else(|| pick(&exact, true))
            .or_else(|| {
                let t = pick(&base, false).or_else(|| pick(&base, true));
                if let Some(t) = t {
                    warnings.push(format!(
                        "Filament {} ({}) mapped to {} — similar material.",
                        f.id, f.kind, t.kind
                    ));
                }
                t
            });
        match chosen {
            Some(t) => {
                taken.push(t.id);
                mapping[f.id as usize - 1] = t.id;
            }
            None => {
                complete = false;
                warnings.push(format!("No {} loaded for filament {} — pick a tray manually.", f.kind, f.id));
                // Closest color of any type keeps the print startable if the
                // user knows better.
                if let Some(t) = pick(&|_| true, false).or_else(|| pick(&|_| true, true)) {
                    taken.push(t.id);
                    mapping[f.id as usize - 1] = t.id;
                }
            }
        }
    }
    finish(mapping, warnings, complete)
}

fn finish(mapping: Vec<i32>, warnings: Vec<String>, complete: bool) -> MappingSuggestion {
    let use_ams = mapping.iter().any(|&m| (0..EXTERNAL_TRAY).contains(&m));
    MappingSuggestion { ams_mapping: mapping, use_ams, warnings, complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fil(id: u32, kind: &str, color: &str) -> SlicedFilament {
        SlicedFilament { id, kind: kind.into(), color: color.into(), used_g: 1.0, used_m: 0.3 }
    }

    fn tray(id: i32, kind: &str, color: &str) -> Tray {
        Tray {
            id,
            ams: Some(id as u32 / 4),
            slot: Some(id as u32 % 4),
            kind: kind.into(),
            color: color.into(),
            name: None,
            remain: None,
            spool_g: None,
            info_idx: None,
            active: false,
            uuid: None,
        }
    }

    fn status(trays: Vec<Tray>) -> StatusView {
        StatusView { has_ams: !trays.is_empty(), trays, ..Default::default() }
    }

    #[test]
    fn picks_type_and_closest_color() {
        let s = status(vec![tray(0, "PLA", "#FFFFFF"), tray(1, "PLA", "#FF0000"), tray(2, "PETG", "#000000")]);
        let m = suggest(&[fil(1, "PLA", "#EE1111"), fil(3, "PETG", "#111111")], 4, &s);
        assert_eq!(m.ams_mapping, vec![1, -1, 2, -1]);
        assert!(m.use_ams);
        assert!(m.complete);
    }

    #[test]
    fn does_not_reuse_a_tray_when_another_fits() {
        let s = status(vec![tray(0, "PLA", "#FFFFFF"), tray(1, "PLA", "#EEEEEE")]);
        let m = suggest(&[fil(1, "PLA", "#FFFFFF"), fil(2, "PLA", "#FFFFFF")], 2, &s);
        assert_eq!(m.ams_mapping.len(), 2);
        assert_ne!(m.ams_mapping[0], m.ams_mapping[1]);
    }

    #[test]
    fn falls_back_to_base_material() {
        let s = status(vec![tray(4, "PLA", "#00FF00")]);
        let m = suggest(&[fil(1, "PLA Silk", "#00EE00")], 1, &s);
        assert_eq!(m.ams_mapping, vec![4]);
        assert!(m.complete);
        assert_eq!(m.warnings.len(), 1);
    }

    #[test]
    fn missing_material_is_flagged() {
        let s = status(vec![tray(0, "PLA", "#000000")]);
        let m = suggest(&[fil(1, "TPU", "#000000")], 1, &s);
        assert!(!m.complete);
        assert_eq!(m.ams_mapping, vec![0]);
    }

    #[test]
    fn no_ams_uses_external_spool() {
        let m = suggest(&[fil(1, "PLA", "#000000")], 1, &StatusView::default());
        assert_eq!(m.ams_mapping, vec![EXTERNAL_TRAY]);
        assert!(!m.use_ams);
    }

    #[test]
    fn pinned_tray_wins_when_still_loaded() {
        let s = status(vec![tray(0, "PLA", "#FFFFFF"), tray(1, "PLA", "#FF0000")]);
        // Closest colour would be tray 0, but the user picked tray 1.
        let m = suggest_with(&[fil(1, "PLA", "#FFFFFF")], 1, &s, &[1]);
        assert_eq!(m.ams_mapping, vec![1]);
        // A pin to a tray with another material is ignored.
        let s2 = status(vec![tray(0, "PLA", "#FFFFFF"), tray(1, "PETG", "#FF0000")]);
        let m2 = suggest_with(&[fil(1, "PLA", "#FFFFFF")], 1, &s2, &[1]);
        assert_eq!(m2.ams_mapping, vec![0]);
    }

    #[test]
    fn external_spool_is_a_candidate() {
        let mut s = status(vec![tray(0, "PLA", "#FFFFFF")]);
        s.external = Some(Tray { id: EXTERNAL_TRAY, ams: None, slot: None, ..tray(0, "TPU", "#000000") });
        let m = suggest(&[fil(1, "TPU", "#000000")], 1, &s);
        assert_eq!(m.ams_mapping, vec![EXTERNAL_TRAY]);
        assert!(!m.use_ams);
    }
}
