//! The print history: every print a printer ran, whoever started it. Prints
//! AirSlice didn't send are noticed from the printer's reports and kept as
//! `Source::External` jobs, and older logs can be imported the same way.

use crate::{
    jobs::{get_job, insert_job, job_dir, new_job, update_job},
    models::*,
    printer::status::{EXTERNAL_TRAY, StatusView},
    state::Shared,
};
use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::BufReader};

/// What the printer is running, when it's a print of its own making: not
/// system tasks such as calibration, which carry no name.
fn external_task(status: &StatusView) -> Option<String> {
    if !status.connected || !matches!(status.state.as_deref(), Some("RUNNING" | "PREPARE" | "PAUSE")) {
        return None;
    }
    if status.print_type.as_deref() == Some("system") {
        return None;
    }
    status.task_name.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

/// When a print began that was first seen well under way (AirSlice was
/// restarted or the printer was out of reach): worked back from what's left.
/// Not while preparing, when the numbers can still be the previous print's,
/// nor in the first few percent, where the printer's estimate is rough.
fn started_at(now: i64, status: &StatusView) -> i64 {
    match (status.state.as_deref(), status.percent, status.remaining_minutes) {
        (Some("RUNNING" | "PAUSE"), Some(p), Some(left)) if (5..100).contains(&p) => now - left * 60 * p / (100 - p),
        _ => now,
    }
}

fn external_file(name: &str) -> ModelFile {
    ModelFile {
        id: String::new(),
        name: name.to_string(),
        kind: "print".into(),
        size_bytes: None,
        thumbnail: None,
        printer: None,
        compatible: vec![],
        plates: None,
        need_ams: None,
        filaments: vec![],
        download_type: None,
        folder: None,
        prediction_secs: None,
        weight_g: None,
    }
}

fn clean_title(task: &str) -> String {
    task.trim_end_matches(".3mf").trim_end_matches(".gcode").to_string()
}

/// Start following any print a printer is running that no job accounts for.
pub async fn notice_external(state: &Shared) {
    let (printers, sending): (Vec<Printer>, bool) = state
        .store
        .read(|d| (d.printers.clone(), d.jobs.iter().any(|j| matches!(j.state, JobState::Uploading | JobState::Starting))))
        .await;
    // A print of ours may show up on the printer before its job says Started.
    if sending {
        return;
    }
    for printer in printers {
        let status = state.printers.status(&printer.id).await;
        let Some(task) = external_task(&status) else { continue };
        let followed = state
            .store
            .read(|d| d.jobs.iter().any(|j| j.state == JobState::Started && j.print.as_ref().is_some_and(|p| p.printer_id == printer.id)))
            .await;
        if followed {
            continue;
        }
        let energy_start = crate::power::energy_kwh(state, &printer).await;
        let title = clean_title(&task);
        let mut job = new_job(Source::External, String::new(), String::new(), title, None, external_file(&task));
        job.state = JobState::Started;
        job.print = Some(PrintStart {
            printer_id: printer.id.clone(),
            remote_file: task.clone(),
            ams_mapping: vec![],
            use_ams: false,
            started_at: started_at(now(), &status),
            seen_running: true,
            outcome: None,
            ended_at: None,
            error_code: None,
            progress: status.percent,
            first_layer_done: status.layer.unwrap_or(0) >= 2,
            energy_start,
            energy_kwh: None,
            layer: status.layer,
            total_layers: status.total_layers,
            waste: None,
        });
        job.push_log(format!("{} is running a print started outside AirSlice", printer.name));
        if insert_job(state, &job).await.is_ok() {
            crate::events::print_started(state, &job).await;
        }
    }
}

/// Purge and prime tower of a multi-colour print, from its sliced file.
pub async fn waste(
    state: &Shared,
    job: &Job,
    slice: &SliceOutcome,
    print: &PrintStart,
    prices: &HashMap<i32, f64>,
    costs: &CostSettings,
) -> Option<PrintWaste> {
    if slice.filaments.iter().filter(|f| f.used_g > 0.0).count() < 2 {
        return None;
    }
    let path = job_dir(state, &job.id).join(job.output_file.as_ref()?);
    let entry = slice.gcode_entry.clone();
    let max_layer = (print.outcome != Some(PrintOutcome::Finished)).then_some(print.layer).flatten();
    let found = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
        let gcode = zip.by_name(&entry)?;
        Ok(crate::gcode::waste(BufReader::new(gcode), max_layer))
    })
    .await
    .ok()?;
    let (mut waste, per_tool) = match found {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(job = %job.id, "couldn't read the purge amounts: {e:#}");
            return None;
        }
    };
    if waste.flush_g + waste.tower_g <= 0.0 {
        return None;
    }
    let use_g: Vec<(i32, f64)> =
        per_tool.into_iter().filter_map(|(tool, g)| print.ams_mapping.get(tool as usize).map(|tray| (*tray, g))).collect();
    let filament_only = CostSettings { per_hour: None, ..costs.clone() };
    waste.cost = crate::spoolman::cost(&use_g, prices, None, &filament_only, true).and_then(|c| c.filament);
    Some(waste)
}

/// A tray as people call it: `A1`, `HT1`, `Ext`.
fn tray_label(tray: i32) -> Option<String> {
    match tray {
        EXTERNAL_TRAY => Some("Ext".into()),
        t if t >= 128 => Some(format!("HT{}", t - 127)),
        t if t >= 0 => Some(format!("{}{}", (b'A' + (t / 4) as u8) as char, t % 4 + 1)),
        _ => None,
    }
}

fn tray_of_label(label: &str) -> i32 {
    if label == "Ext" {
        return EXTERNAL_TRAY;
    }
    if let Some(n) = label.strip_prefix("HT").and_then(|n| n.parse::<i32>().ok()) {
        return n + 127;
    }
    let mut c = label.chars();
    match (c.next(), c.as_str().parse::<i32>()) {
        (Some(l @ 'A'..='Z'), Ok(n)) if n >= 1 => (l as i32 - 'A' as i32) * 4 + n - 1,
        _ => -1,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PrintUse {
    pub slot: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub color: String,
    pub grams: f64,
}

/// A print as the history shows it.
#[derive(Debug, Clone, Serialize)]
pub struct PrintRecord {
    pub id: String,
    pub name: String,
    pub printer_id: String,
    /// Not started from AirSlice.
    pub external: bool,
    pub outcome: PrintOutcome,
    pub started_at: i64,
    pub ended_at: i64,
    pub progress: Option<i64>,
    pub layer: Option<i64>,
    pub total_layers: Option<i64>,
    pub error: Option<String>,
    /// What it used; a stopped print only up to where it got.
    pub weight_g: Option<f64>,
    pub usage: Vec<PrintUse>,
    pub cost: Option<Cost>,
    pub waste: Option<PrintWaste>,
    pub energy_kwh: Option<f64>,
    pub photo: bool,
    pub cover: Option<String>,
    pub model_url: Option<String>,
}

/// Imported prints already say what they really used.
fn usage_share(job: &Job, print: &PrintStart) -> f64 {
    let imported = job.slice.as_ref().is_some_and(|s| s.settings == IMPORTED);
    match print.outcome {
        Some(PrintOutcome::Failed | PrintOutcome::Cancelled) if !imported => print.progress.unwrap_or(0).clamp(0, 100) as f64 / 100.0,
        _ => 1.0,
    }
}

fn record(job: &Job) -> Option<PrintRecord> {
    let print = job.print.as_ref()?;
    let outcome = print.outcome?;
    let share = usage_share(job, print);
    let usage: Vec<PrintUse> = job
        .slice
        .iter()
        .flat_map(|s| &s.filaments)
        .filter(|f| f.used_g > 0.0)
        .map(|f| PrintUse {
            slot: print.ams_mapping.get(f.id as usize - 1).copied().and_then(tray_label),
            kind: f.kind.clone(),
            color: f.color.clone(),
            grams: (f.used_g * share * 100.0).round() / 100.0,
        })
        .collect();
    let weight_g = if usage.is_empty() {
        job.slice.as_ref().and_then(|s| s.weight_g).map(|g| (g * share * 100.0).round() / 100.0)
    } else {
        Some((usage.iter().map(|u| u.grams).sum::<f64>() * 100.0).round() / 100.0)
    };
    let name = match &job.plate_label {
        Some(plate) => format!("{}, {plate}", job.model_title),
        None => job.model_title.clone(),
    };
    Some(PrintRecord {
        id: job.id.clone(),
        name,
        printer_id: print.printer_id.clone(),
        external: job.source == Source::External,
        outcome,
        started_at: print.started_at,
        ended_at: print.ended_at.unwrap_or(job.updated_at),
        progress: print.progress,
        layer: print.layer,
        total_layers: print.total_layers,
        error: job.error.clone(),
        weight_g,
        usage,
        cost: job.cost.clone(),
        waste: print.waste.clone(),
        energy_kwh: print.energy_kwh,
        photo: job.photo,
        cover: job.model_cover.clone(),
        model_url: Some(job.model_url.clone()).filter(|u| !u.is_empty()),
    })
}

pub async fn list(state: &Shared, printer_id: Option<&str>, limit: usize) -> Vec<PrintRecord> {
    let mut list: Vec<PrintRecord> = state
        .store
        .read(|d| d.jobs.iter().filter_map(record).filter(|r| printer_id.is_none_or(|p| p == r.printer_id)).collect())
        .await;
    list.sort_by_key(|r| std::cmp::Reverse(r.ended_at));
    list.truncate(limit);
    list
}

/// `SliceOutcome::settings` of an imported print, which never was sliced here.
const IMPORTED: &str = "imported";

#[derive(Debug, Deserialize)]
pub struct ImportUse {
    #[serde(default)]
    slot: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    color: String,
    grams: f64,
}

/// A print from an older log. Importing the same `key` twice does nothing.
#[derive(Debug, Deserialize)]
pub struct ImportPrint {
    key: String,
    name: String,
    /// The name the printer showed, to spot a print AirSlice already has.
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    printer_id: Option<String>,
    outcome: PrintOutcome,
    started_at: i64,
    ended_at: i64,
    #[serde(default)]
    progress: Option<i64>,
    #[serde(default)]
    layer: Option<i64>,
    #[serde(default)]
    total_layers: Option<i64>,
    #[serde(default)]
    weight_g: Option<f64>,
    #[serde(default)]
    usage: Vec<ImportUse>,
    #[serde(default)]
    cost: Option<Cost>,
    #[serde(default)]
    waste: Option<PrintWaste>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub id: String,
    /// `imported`, or `exists`: this key or the same print is already there.
    pub status: &'static str,
}

/// The same print: the printer's name for it, ended within a quarter of an hour.
fn same_print(job: &Job, task: &str, ended_at: i64) -> bool {
    job.print.as_ref().is_some_and(|p| p.is_task(Some(task)) && p.ended_at.is_some_and(|e| (e - ended_at).abs() < 900))
}

pub async fn import(state: &Shared, body: ImportPrint) -> anyhow::Result<ImportResult> {
    if body.key.trim().is_empty() || body.name.trim().is_empty() {
        bail!("an imported print needs a key and a name");
    }
    if body.ended_at < body.started_at {
        bail!("an imported print can't end before it starts");
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, body.key.as_bytes());
    let id: String = digest.as_ref()[..6].iter().map(|b| format!("{b:02x}")).collect();
    let task = body.task.clone().unwrap_or_else(|| body.name.clone());
    let (exists, default_printer) = state
        .store
        .read(|d| {
            let found = d.jobs.iter().find(|j| j.id == id || same_print(j, &task, body.ended_at)).map(|j| j.id.clone());
            (found, d.default_printer_id.clone().or_else(|| d.printers.first().map(|p| p.id.clone())))
        })
        .await;
    if let Some(id) = exists {
        return Ok(ImportResult { id, status: "exists" });
    }
    let printer_id = body.printer_id.clone().or(default_printer).ok_or_else(|| anyhow!("add a printer first"))?;

    let mut job = new_job(Source::External, String::new(), String::new(), body.name.clone(), None, external_file(&task));
    job.id = id.clone();
    job.created_at = body.started_at;
    job.updated_at = body.ended_at;
    job.files_removed = true;
    job.cost = body.cost;
    let mut mapping = Vec::new();
    let filaments = body
        .usage
        .iter()
        .enumerate()
        .map(|(i, u)| {
            mapping.push(tray_of_label(&u.slot));
            SlicedFilament { id: i as u32 + 1, kind: u.kind.clone(), color: u.color.clone(), used_g: u.grams, used_m: 0.0 }
        })
        .collect::<Vec<_>>();
    job.slice = Some(SliceOutcome {
        printer_id: printer_id.clone(),
        plate: 1,
        gcode_entry: String::new(),
        filament_slots: filaments.len() as u32,
        filaments,
        prediction_secs: None,
        weight_g: body.weight_g,
        settings: IMPORTED.into(),
        transform: None,
        filament_profile: None,
        preferred_trays: None,
        bed_type: None,
        tweaks: Default::default(),
        options: None,
        output_path: Default::default(),
    });
    job.print = Some(PrintStart {
        printer_id,
        remote_file: task,
        ams_mapping: mapping,
        use_ams: false,
        started_at: body.started_at,
        seen_running: true,
        outcome: Some(body.outcome),
        ended_at: Some(body.ended_at),
        error_code: None,
        progress: body.progress,
        first_layer_done: true,
        energy_start: None,
        energy_kwh: None,
        layer: body.layer,
        total_layers: body.total_layers,
        waste: body.waste,
    });
    job.state = match body.outcome {
        PrintOutcome::Finished | PrintOutcome::Unknown => JobState::Finished,
        PrintOutcome::Failed | PrintOutcome::Cancelled => JobState::Failed,
    };
    job.log.push("Imported from an older print log".into());
    state
        .store
        .update(|d| {
            let at = d.jobs.iter().position(|j| j.created_at < job.created_at).unwrap_or(d.jobs.len());
            d.jobs.insert(at, job);
        })
        .await?;
    Ok(ImportResult { id, status: "imported" })
}

/// Give an external print its picture.
pub async fn set_photo(state: &Shared, id: &str, bytes: &[u8]) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.source != Source::External {
        bail!("only a print started outside AirSlice can be given a picture");
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        bail!("the picture must be a JPEG");
    }
    let dir = state.cfg.data_dir.join("photos");
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(format!("{}.jpg", job.id)), bytes).await?;
    update_job(state, id, |j| j.photo = true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy(state: &str, task: &str) -> StatusView {
        StatusView { connected: true, state: Some(state.into()), task_name: Some(task.into()), ..Default::default() }
    }

    #[test]
    fn only_running_named_prints_count_as_external() {
        assert_eq!(external_task(&busy("RUNNING", " Egg ")).as_deref(), Some("Egg"));
        assert_eq!(external_task(&busy("PAUSE", "Egg")).as_deref(), Some("Egg"));
        assert!(external_task(&busy("FINISH", "Egg")).is_none());
        assert!(external_task(&busy("IDLE", "Egg")).is_none());
        assert!(external_task(&busy("SLICING", "Egg")).is_none());
        assert!(external_task(&busy("RUNNING", "")).is_none());
        assert!(external_task(&StatusView { print_type: Some("system".into()), ..busy("RUNNING", "Cali") }).is_none());
        assert!(external_task(&StatusView { connected: false, ..busy("RUNNING", "Egg") }).is_none());
    }

    #[test]
    fn a_print_seen_late_is_dated_back() {
        let at = |state: &str, percent, left| StatusView { percent, remaining_minutes: left, ..busy(state, "Egg") };
        // Half way with an hour left: it began an hour ago.
        assert_eq!(started_at(10_000, &at("RUNNING", Some(50), Some(60))), 6_400);
        assert_eq!(started_at(10_000, &at("RUNNING", None, None)), 10_000);
        assert_eq!(started_at(10_000, &at("RUNNING", Some(1), Some(300))), 10_000, "the start of a print is now");
        assert_eq!(started_at(10_000, &at("PREPARE", Some(99), Some(5))), 10_000, "the previous print's numbers");
    }

    #[test]
    fn tray_labels_round_trip() {
        for (tray, label) in [(0, "A1"), (3, "A4"), (5, "B2"), (128, "HT1"), (129, "HT2"), (EXTERNAL_TRAY, "Ext")] {
            assert_eq!(tray_label(tray).as_deref(), Some(label));
            assert_eq!(tray_of_label(label), tray);
        }
        assert_eq!(tray_label(-1), None);
        assert_eq!(tray_of_label("?"), -1);
    }

    fn job_with(outcome: PrintOutcome, progress: i64, settings: &str) -> Job {
        let mut job = new_job(Source::Upload, String::new(), String::new(), "Egg".into(), None, external_file("Egg"));
        job.slice = Some(SliceOutcome {
            printer_id: "p".into(),
            plate: 1,
            gcode_entry: String::new(),
            filament_slots: 2,
            filaments: vec![
                SlicedFilament { id: 1, kind: "PLA".into(), color: "#FFFFFF".into(), used_g: 100.0, used_m: 1.0 },
                SlicedFilament { id: 2, kind: "PLA".into(), color: "#000000".into(), used_g: 0.0, used_m: 0.0 },
            ],
            prediction_secs: None,
            weight_g: Some(100.0),
            settings: settings.into(),
            transform: None,
            filament_profile: None,
            preferred_trays: None,
            bed_type: None,
            tweaks: Default::default(),
            options: None,
            output_path: Default::default(),
        });
        job.print = Some(PrintStart {
            printer_id: "p".into(),
            remote_file: "Egg_abc.3mf".into(),
            ams_mapping: vec![1, 2],
            use_ams: true,
            started_at: 100,
            seen_running: true,
            outcome: Some(outcome),
            ended_at: Some(200),
            error_code: None,
            progress: Some(progress),
            first_layer_done: true,
            energy_start: None,
            energy_kwh: None,
            layer: None,
            total_layers: None,
            waste: None,
        });
        job
    }

    #[test]
    fn a_stopped_print_counts_what_it_used() {
        let r = record(&job_with(PrintOutcome::Failed, 25, "printer")).unwrap();
        assert_eq!(r.weight_g, Some(25.0));
        assert_eq!(r.usage.len(), 1, "unused filaments are left out");
        assert_eq!(r.usage[0].slot.as_deref(), Some("A2"));
        let r = record(&job_with(PrintOutcome::Finished, 100, "printer")).unwrap();
        assert_eq!(r.weight_g, Some(100.0));
        // An imported log already holds what was really used.
        let r = record(&job_with(PrintOutcome::Failed, 25, IMPORTED)).unwrap();
        assert_eq!(r.weight_g, Some(100.0));
    }

    #[test]
    fn a_print_still_running_is_not_in_the_history() {
        let mut job = job_with(PrintOutcome::Finished, 100, "printer");
        job.print.as_mut().unwrap().outcome = None;
        assert!(record(&job).is_none());
    }

    #[test]
    fn same_print_needs_the_name_and_a_close_end() {
        let job = job_with(PrintOutcome::Finished, 100, "printer");
        assert!(same_print(&job, "Egg_abc", 300));
        assert!(same_print(&job, "Egg_abc.gcode.3mf", 300));
        assert!(!same_print(&job, "Other", 300));
        assert!(!same_print(&job, "Egg_abc", 2000));
    }

    #[test]
    fn waste_counts_purge_and_tower_per_filament() {
        let gcode = "\
; filament_density: 1.0,2.0
; filament_diameter: 2.0,2.0
M620 S0A
G1 X1 E100
T1
; FLUSH_START
G1 X1 E10
G1 E-5
; FLUSH_END
G1 X2 E100
; FEATURE: Prime tower
G1 X3 E20
; FEATURE: Outer wall
G1 X4 E100
M73 L3
T0
; FLUSH_START
G1 X1 E30
; FLUSH_END
";
        let mm3 = std::f64::consts::PI; // per mm of 2 mm filament
        let round = |g: f64| (g * 100.0).round() / 100.0;
        let (w, tools) = crate::gcode::waste(gcode.as_bytes(), None);
        assert_eq!(w.changes, 2);
        assert_eq!(w.flush_g, round((10.0 * 2.0 + 30.0) * mm3 / 1000.0));
        assert_eq!(w.tower_g, round(20.0 * 2.0 * mm3 / 1000.0));
        assert_eq!(tools.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0, 1]);
        // A print that stopped at layer 2 never got to the second purge.
        let (w, _) = crate::gcode::waste(gcode.as_bytes(), Some(2));
        assert_eq!(w.changes, 1);
        assert_eq!(w.flush_g, round(10.0 * 2.0 * mm3 / 1000.0));
    }
}
