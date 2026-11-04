//! OrcaSlicer command line + its bundled Bambu Lab profiles.

use crate::{config::Config, models::SliceProgress};
use anyhow::{Context, anyhow, bail};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::{OnceCell, Semaphore}};

#[derive(Debug, Clone, Serialize)]
pub struct MachineProfile {
    pub name: String,
    pub printer_model: String,
    pub nozzle: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament_type: Option<String>,
    /// Bambu filament id (e.g. "GFA00"), matches the spool RFID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProfileChoices {
    pub processes: Vec<NamedProfile>,
    pub filaments: Vec<NamedProfile>,
    pub default_process: Option<String>,
}

/// A build plate OrcaSlicer knows: its setting value, the name Bambu Lab
/// sells it under, and the filament setting that holds its bed temperature
/// (0 = the filament can't go on it).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BuildPlate {
    pub value: &'static str,
    pub label: &'static str,
    #[serde(skip)]
    temp_key: &'static str,
}

/// In order of preference for an automatic choice.
pub const BUILD_PLATES: [BuildPlate; 6] = [
    BuildPlate { value: "Textured PEI Plate", label: "Textured PEI Plate", temp_key: "textured_plate_temp" },
    BuildPlate { value: "Cool Plate", label: "Cool Plate (smooth)", temp_key: "cool_plate_temp" },
    BuildPlate { value: "Supertack Plate", label: "Cool Plate SuperTack", temp_key: "supertack_plate_temp" },
    BuildPlate { value: "Textured Cool Plate", label: "Textured Cool Plate", temp_key: "textured_cool_plate_temp" },
    BuildPlate { value: "Engineering Plate", label: "Engineering Plate", temp_key: "eng_plate_temp" },
    BuildPlate { value: "High Temp Plate", label: "High Temperature Plate (smooth PEI)", temp_key: "hot_plate_temp" },
];

/// The plate with this setting value (any case; "SuperTack Plate" too).
pub fn build_plate(value: &str) -> Option<&'static BuildPlate> {
    BUILD_PLATES.iter().find(|p| p.value.eq_ignore_ascii_case(value.trim()))
}

pub struct FilamentFacts {
    pub kind: String,
    pub filament_id: String,
    pub temp_min: u32,
    pub temp_max: u32,
}

#[derive(Default)]
pub struct ProfileIndex {
    machines: HashMap<String, Value>,
    processes: HashMap<String, Value>,
    filaments: HashMap<String, Value>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => s.split(';').filter(|x| !x.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    }
}

fn first_string(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => a.first().and_then(|x| x.as_str()).map(str::to_string),
        _ => None,
    }
}

impl ProfileIndex {
    /// Load the Bambu Lab vendor profiles and resolve inheritance, the way
    /// the slicer GUI does before handing settings to the engine.
    pub fn load(profiles_dir: &Path) -> anyhow::Result<Self> {
        let library = load_vendor(profiles_dir, "OrcaFilamentLibrary").unwrap_or_default();
        let bbl = load_vendor(profiles_dir, "BBL")
            .with_context(|| format!("Bambu Lab profiles not found in {}", profiles_dir.display()))?;
        let mut index = Self::default();
        for (cat, raw) in [("machine", &bbl.0), ("process", &bbl.1), ("filament", &bbl.2)] {
            let fallback = if cat == "filament" { Some(&library.2) } else { None };
            for (name, profile) in raw {
                if profile.get("instantiation").and_then(|v| v.as_str()) != Some("true") {
                    continue;
                }
                let resolved = resolve(profile, raw, fallback, 0);
                let target = match cat {
                    "machine" => &mut index.machines,
                    "process" => &mut index.processes,
                    _ => &mut index.filaments,
                };
                target.insert(name.clone(), resolved);
            }
        }
        if index.machines.is_empty() {
            bail!("no Bambu Lab machine profiles found");
        }
        Ok(index)
    }

    pub fn machines(&self) -> Vec<MachineProfile> {
        let mut out: Vec<MachineProfile> = self
            .machines
            .iter()
            .map(|(name, v)| MachineProfile {
                name: name.clone(),
                printer_model: v.get("printer_model").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                nozzle: first_string(v.get("nozzle_diameter")).unwrap_or_default(),
            })
            .collect();
        out.sort_by(|a, b| a.printer_model.cmp(&b.printer_model).then(a.nozzle.cmp(&b.nozzle)));
        out
    }

    /// Bed of a machine profile, from `printable_area` / `printable_height`.
    pub fn bed(&self, machine: &str) -> Option<crate::preview::Bed> {
        let m = self.machines.get(machine)?;
        let height = m.get("printable_height").and_then(|h| h.as_str()?.parse().ok());
        crate::preview::Bed::from_settings(&strings(m.get("printable_area")), height)
    }

    pub fn has_machine(&self, name: &str) -> bool {
        self.machines.contains_key(name)
    }

    pub fn choices_for(&self, machine: &str) -> ProfileChoices {
        let compatible = |v: &Value| {
            let list = strings(v.get("compatible_printers"));
            list.is_empty() || list.iter().any(|m| m == machine)
        };
        let mut processes: Vec<NamedProfile> = self
            .processes
            .iter()
            .filter(|(_, v)| compatible(v))
            .map(|(n, _)| NamedProfile { name: n.clone(), filament_type: None, filament_id: None })
            .collect();
        processes.sort_by(|a, b| a.name.cmp(&b.name));
        let mut filaments: Vec<NamedProfile> = self
            .filaments
            .iter()
            .filter(|(_, v)| compatible(v))
            .map(|(n, v)| NamedProfile {
                name: n.clone(),
                filament_type: first_string(v.get("filament_type")),
                filament_id: first_string(v.get("filament_id")).filter(|s| !s.is_empty()),
            })
            .collect();
        filaments.sort_by(|a, b| a.name.cmp(&b.name));
        let default_process = self
            .machines
            .get(machine)
            .and_then(|m| m.get("default_print_profile"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|p| processes.iter().any(|x| &x.name == p))
            .or_else(|| processes.iter().find(|p| p.name.contains("0.20mm Standard")).map(|p| p.name.clone()));
        ProfileChoices { processes, filaments, default_process }
    }

    /// Default filament for a machine ("Bambu PLA Basic" or the machine's own default).
    pub fn default_filament(&self, machine: &str) -> Option<String> {
        let choices = self.choices_for(machine);
        let from_machine = self
            .machines
            .get(machine)
            .map(|m| strings(m.get("default_filament_profile")))
            .unwrap_or_default()
            .into_iter()
            .find(|f| choices.filaments.iter().any(|x| &x.name == f));
        from_machine
            .or_else(|| choices.filaments.iter().find(|f| f.name.starts_with("Bambu PLA Basic")).map(|f| f.name.clone()))
            .or_else(|| choices.filaments.iter().find(|f| f.name.starts_with("Generic PLA")).map(|f| f.name.clone()))
    }

    /// Filament profile for a loaded AMS tray: the exact Bambu profile when
    /// the spool has an RFID id, otherwise the best profile for its material.
    pub fn filament_for_tray(&self, machine: &str, info_idx: Option<&str>, tray_type: &str) -> Option<String> {
        let filaments = self.choices_for(machine).filaments;
        if let Some(id) = info_idx.filter(|i| !i.is_empty())
            && let Some(f) = filaments.iter().find(|f| f.filament_id.as_deref() == Some(id)) {
                return Some(f.name.clone());
            }
        let t = tray_type.trim();
        if t.is_empty() {
            return None;
        }
        let same: Vec<&NamedProfile> = filaments
            .iter()
            .filter(|f| f.filament_type.as_deref().map(|ft| ft.eq_ignore_ascii_case(t)).unwrap_or(false))
            .collect();
        let starts = |prefix: String| same.iter().find(|f| f.name.to_lowercase().starts_with(&prefix.to_lowercase())).map(|f| f.name.clone());
        starts(format!("Bambu {t} Basic"))
            .or_else(|| starts(format!("Generic {t} ")))
            .or_else(|| starts(format!("Generic {t}")))
            .or_else(|| starts(format!("Bambu {t}")))
            .or_else(|| same.first().map(|f| f.name.clone()))
    }

    /// What the printer needs to set a tray to this filament profile.
    pub fn filament_facts(&self, machine: &str, name: &str) -> Option<FilamentFacts> {
        let choice = self.choices_for(machine).filaments.into_iter().find(|f| f.name == name)?;
        let v = self.filaments.get(name)?;
        let temp = |k: &str| first_string(v.get(k)).and_then(|t| t.trim().parse::<f64>().ok()).map(|t| t as u32);
        Some(FilamentFacts {
            kind: choice.filament_type.unwrap_or_default(),
            filament_id: choice.filament_id.unwrap_or_default(),
            temp_min: temp("nozzle_temperature_range_low").unwrap_or(190),
            temp_max: temp("nozzle_temperature_range_high").unwrap_or(240),
        })
    }

    /// Whether every one of these filament profiles can go on the plate.
    /// Unknown plates and profiles without a temperature for it count as yes.
    pub fn takes(&self, plate: &str, profiles: &[&str]) -> bool {
        self.refused_by(plate, profiles).is_none()
    }

    /// The first profile that can't go on the plate.
    fn refused_by<'p>(&self, plate: &str, profiles: &[&'p str]) -> Option<&'p str> {
        let key = build_plate(plate)?.temp_key;
        profiles.iter().copied().find(|name| {
            self.filaments
                .get(*name)
                .and_then(|f| first_string(f.get(key)))
                .and_then(|t| t.trim().parse::<f64>().ok())
                .map(|t| t <= 0.0)
                .unwrap_or(false)
        })
    }

    /// A build plate every one of these filament profiles takes, when the
    /// `current` one doesn't take them all: one of `owned` when the printer
    /// lists its plates, else any. `None` when the current plate is fine (or
    /// no plate would do).
    pub fn plate_for(&self, current: &str, profiles: &[&str], owned: &[String]) -> Option<&'static str> {
        if self.takes(current, profiles) {
            return None;
        }
        BUILD_PLATES
            .iter()
            .filter(|p| owned.is_empty() || owned.iter().any(|o| o.eq_ignore_ascii_case(p.value)))
            .map(|p| p.value)
            .find(|p| self.takes(p, profiles))
    }

    /// Write a filament profile for slicing, in the colour of the tray it
    /// will print from, so the preview and the printer show that colour.
    fn write_filament(&self, name: &str, color: Option<&str>, dest: &Path) -> anyhow::Result<()> {
        let mut v = self.filaments.get(name).cloned().ok_or_else(|| anyhow!("profile {name:?} not found"))?;
        if let (Some(c), Some(map)) = (color.filter(|c| !c.is_empty()), v.as_object_mut()) {
            map.insert("filament_colour".into(), Value::Array(vec![Value::String(c.to_string())]));
        }
        std::fs::write(dest, serde_json::to_vec_pretty(&v)?)?;
        Ok(())
    }

    fn write(&self, map: &HashMap<String, Value>, name: &str, dest: &Path) -> anyhow::Result<()> {
        let v = map.get(name).ok_or_else(|| anyhow!("profile {name:?} not found"))?;
        std::fs::write(dest, serde_json::to_vec_pretty(v)?)?;
        Ok(())
    }
}

type Vendor = (HashMap<String, Value>, HashMap<String, Value>, HashMap<String, Value>);

fn load_vendor(dir: &Path, vendor: &str) -> anyhow::Result<Vendor> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{vendor}.json")))?)?;
    let load = |key: &str| -> HashMap<String, Value> {
        let mut out = HashMap::new();
        for entry in manifest.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
            let (Some(name), Some(sub)) = (entry.get("name").and_then(|v| v.as_str()), entry.get("sub_path").and_then(|v| v.as_str())) else { continue };
            match std::fs::read(dir.join(vendor).join(sub)).map(|b| serde_json::from_slice::<Value>(&b)) {
                Ok(Ok(v)) => {
                    out.insert(name.to_string(), v);
                }
                _ => tracing::debug!("skipping unreadable profile {vendor}/{sub}"),
            }
        }
        out
    };
    Ok((load("machine_list"), load("process_list"), load("filament_list")))
}

fn resolve(profile: &Value, siblings: &HashMap<String, Value>, fallback: Option<&HashMap<String, Value>>, depth: u8) -> Value {
    let Some(parent_name) = profile.get("inherits").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
        return profile.clone();
    };
    let parent = siblings.get(parent_name).or_else(|| fallback.and_then(|f| f.get(parent_name)));
    let (Some(parent), true) = (parent, depth < 16) else { return profile.clone() };
    let mut merged = match resolve(parent, siblings, fallback, depth + 1) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(child) = profile {
        for (k, v) in child {
            merged.insert(k.clone(), v.clone());
        }
    }
    Value::Object(merged)
}

pub struct SliceRequest<'a> {
    pub input: &'a Path,
    pub work_dir: &'a Path,
    pub plate: u32,
    /// Override the project's printer/process with these profiles.
    pub machine: Option<&'a str>,
    pub process: Option<&'a str>,
    /// Build plate to slice for (`curr_bed_type` value); `None` keeps the
    /// profile's or project's own unless the filaments can't go on it.
    pub bed_type: Option<&'a str>,
    /// The printer's plates, for that automatic choice; empty = any.
    pub plates: &'a [String],
    /// Settings to change in the process (or the project's settings).
    pub overrides: &'a [(String, String)],
    /// Called with OrcaSlicer's progress while it runs.
    pub progress: Option<&'a (dyn Fn(SliceProgress) + Send + Sync)>,
    /// Filament profiles to slice with, one per project filament slot.
    /// Empty: the project's own filaments.
    pub filaments: &'a [SliceFilament],
    /// Auto-arrange on the plate (for bare meshes).
    pub arrange: bool,
    /// Auto-orient (for bare meshes).
    pub orient: bool,
    /// Scale factor (1.0 = unchanged).
    pub scale: Option<f64>,
    /// Rotation around Z in degrees.
    pub rotate_z: Option<f64>,
    /// Total copies on the plate.
    pub copies: u32,
    /// Other models to arrange on the plate with this one.
    pub extras: &'a [SliceExtra],
}

/// A model added to the plate.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceExtra {
    pub path: PathBuf,
    /// A project's plate to take the objects from.
    pub plate: u32,
    pub copies: u32,
    /// Filament slot (1-based) to print it with.
    pub slot: u32,
    /// Project files keep their orientation; a bare mesh is laid flat.
    pub orient: bool,
}

/// Files of added models are named this, so they can be told apart in the
/// combined project.
fn extra_file_name(extra: usize, part: usize) -> String {
    format!("airslice-e{extra}_{part}.stl")
}

fn extra_of_file(name: &str) -> Option<usize> {
    name.strip_prefix("airslice-e")?.split('_').next()?.parse().ok()
}

/// One filament to slice with.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceFilament {
    pub profile: String,
    /// `#RRGGBB`; the profile's own colour when `None`.
    pub color: Option<String>,
}

pub struct SliceOutput {
    pub output: PathBuf,
    pub log_tail: Vec<String>,
    /// Project settings dropped because OrcaSlicer refused their values.
    pub adjusted: Vec<String>,
    /// Other things AirSlice changed to make the slice work, for the job log.
    pub notes: Vec<String>,
    /// The plate in the output, when it isn't the one asked for.
    pub plate: Option<u32>,
    /// Objects of added models that should be on the plate.
    pub extra_objects: u32,
}

pub struct Slicer {
    cfg: Config,
    index: OnceCell<Result<Arc<ProfileIndex>, String>>,
    gate: Semaphore,
}

impl Slicer {
    pub fn new(cfg: Config) -> Self {
        Self { cfg, index: OnceCell::new(), gate: Semaphore::new(1) }
    }

    pub fn available(&self) -> bool {
        self.cfg.orca_path.exists()
    }

    pub fn binary(&self) -> &Path {
        &self.cfg.orca_path
    }

    pub async fn profiles(&self) -> anyhow::Result<Arc<ProfileIndex>> {
        let dir = self.cfg.orca_profiles_dir.clone();
        self.index
            .get_or_init(|| async move {
                tokio::task::spawn_blocking(move || ProfileIndex::load(&dir).map(Arc::new).map_err(|e| format!("{e:#}")))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            })
            .await
            .clone()
            .map_err(|e| anyhow!(e))
    }

    pub async fn version(&self) -> Option<String> {
        let dir = self.cfg.orca_path.parent()?;
        // The AppImage ships a version file in resources; fall back to env.
        if let Ok(v) = std::env::var("ORCA_VERSION") {
            return Some(v);
        }
        tokio::fs::read_to_string(dir.join("resources").join("version.txt")).await.ok().map(|s| s.trim().to_string())
    }

    pub async fn slice(&self, req: SliceRequest<'_>) -> anyhow::Result<SliceOutput> {
        if !self.available() {
            bail!("OrcaSlicer was not found at {} (set ORCA_SLICER_PATH)", self.cfg.orca_path.display());
        }
        let _permit = self.gate.acquire().await?;
        let out_dir = req.work_dir.join("out");
        reset_dir(&out_dir).await?;
        let mut input = req.input.to_path_buf();
        let mut notes = Vec::new();

        // Profiles, written next to the job.
        let mut settings_args: Vec<String> = Vec::new();
        let mut filament_args: Vec<String> = Vec::new();
        let mut process_path = None;
        let index = if req.machine.is_some() || !req.filaments.is_empty() { Some(self.profiles().await?) } else { None };
        if let Some(index) = &index {
            let pdir = req.work_dir.join("profiles");
            reset_dir(&pdir).await?;
            if let Some(machine) = req.machine {
                let mpath = pdir.join("machine.json");
                index.write(&index.machines, machine, &mpath)?;
                let mut settings = mpath.to_string_lossy().into_owned();
                if let Some(process) = req.process {
                    let ppath = pdir.join("process.json");
                    index.write(&index.processes, process, &ppath)?;
                    settings.push(';');
                    settings.push_str(&ppath.to_string_lossy());
                    process_path = Some(ppath);
                }
                settings_args.extend(["--load-settings".into(), settings]);
            }
            if !req.filaments.is_empty() {
                let mut paths = Vec::new();
                for (i, f) in req.filaments.iter().enumerate() {
                    let fpath = pdir.join(format!("filament_{}.json", i + 1));
                    index.write_filament(&f.profile, f.color.as_deref(), &fpath)?;
                    paths.push(fpath.to_string_lossy().into_owned());
                }
                filament_args.extend(["--load-filaments".into(), paths.join(";")]);
            }
        }

        // The build plate: the one asked for, else the profile's or project's
        // own, unless a chosen filament can't go on it (PETG on the Cool
        // Plate stops OrcaSlicer); then one of the printer's plates that takes
        // them all.
        let profiles: Vec<&str> = req.filaments.iter().map(|f| f.profile.as_str()).collect();
        let current = match &process_path {
            Some(p) => json_setting(&tokio::fs::read(p).await?, BED_TYPE),
            None if is_3mf(&input) => crate::threemf::project_setting(&input, BED_TYPE),
            None => None,
        }
        .unwrap_or_else(|| DEFAULT_PLATE.to_string());
        let target = match req.bed_type {
            Some(plate) => {
                if let Some(refused) = index.as_ref().and_then(|i| i.refused_by(plate, &profiles)) {
                    bail!(
                        "{} can't go on the {}. Choose another build plate or another tray.",
                        refused.split(" @").next().unwrap_or(refused),
                        plate_label(plate)
                    );
                }
                (!plate.eq_ignore_ascii_case(&current)).then(|| plate.to_string())
            }
            None => index.as_ref().and_then(|i| i.plate_for(&current, &profiles, req.plates)).map(|plate| {
                notes.push(format!(
                    "sliced for the {}: the {} doesn't take the chosen filament",
                    plate_label(plate),
                    plate_label(&current)
                ));
                plate.to_string()
            }),
        };
        let mut set: Vec<(String, String)> = req.overrides.to_vec();
        if let Some(plate) = target {
            set.push((BED_TYPE.to_string(), plate));
        }
        if !set.is_empty() {
            match &process_path {
                Some(p) => {
                    for (k, v) in &set {
                        set_json_setting(p, k, v).await?;
                    }
                }
                None if is_3mf(&input) => {
                    let dest = req.work_dir.join("input-plate.3mf");
                    let (src, d) = (input.clone(), dest.clone());
                    tokio::task::spawn_blocking(move || crate::threemf::edit_settings(&src, &d, &[], &set)).await??;
                    input = dest;
                }
                None => {}
            }
        }

        let profile_args: Vec<String> = settings_args.iter().chain(&filament_args).cloned().collect();
        let mesh = !is_3mf(req.input);
        let mut transform = Transform::of(&req);
        let mut plate = req.plate;
        let mut inputs: Vec<String> = Vec::new();
        let mut id_args: Vec<String> = Vec::new();
        let mut extra_objects = 0;
        if !req.extras.is_empty() {
            let meshes = self.extra_meshes(&req, &settings_args).await?;
            extra_objects = meshes.iter().map(|m| m.copies).sum();
            let work = req.work_dir.join("extras-work");
            if mesh {
                // Meshes take a filament each on OrcaSlicer's command line.
                let base = self.export_stl(&input, &work.join("base"), req.work_dir, &settings_args, &Transform { copies: 1, arrange: false, ..transform }.shape_args()).await?;
                let mut ids = Vec::new();
                for _ in 0..req.copies.max(1) {
                    for (_, path) in &base {
                        inputs.push(path.to_string_lossy().into_owned());
                        ids.push(1);
                    }
                }
                for m in &meshes {
                    for _ in 0..m.copies {
                        inputs.push(m.path.to_string_lossy().into_owned());
                        ids.push(m.slot);
                    }
                }
                if ids.iter().any(|s| *s != 1) {
                    id_args = vec!["--load-filament-ids".into(), ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")];
                }
                transform = Transform { arrange: true, ..Transform::default() };
            } else {
                input = self.combine(&req, &input, &meshes, &settings_args, &work).await?;
                transform = Transform::default();
            }
            plate = 1;
        }

        // OrcaSlicer only repeats projects (--repetitions fails on a bare
        // mesh), so make the mesh a project first, with its size and turn.
        if mesh && req.copies > 1 && req.extras.is_empty() {
            let prep = req.work_dir.join("mesh");
            reset_dir(&prep).await?;
            let mut args = vec!["--export-3mf".to_string(), "mesh.3mf".into()];
            args.extend(Transform { copies: 1, ..transform }.args());
            args.extend(profile_args.iter().cloned());
            args.extend(["--allow-newer-file".into(), "--outputdir".into(), prep.to_string_lossy().into_owned()]);
            args.push(input.to_string_lossy().into_owned());
            let run = self.run(&args, req.work_dir, &prep, None).await?;
            let project = prep.join("mesh.3mf");
            if run.error.is_some() || !project.exists() {
                bail!(
                    "OrcaSlicer couldn't lay out the copies: {}",
                    run.error.or_else(|| run.log_tail.last().cloned()).unwrap_or_default()
                );
            }
            input = project;
            transform = Transform { copies: req.copies, arrange: true, ..Transform::default() };
        }

        let mut args: Vec<String> = vec!["--slice".into(), plate.to_string()];
        args.extend(["--export-3mf".into(), "sliced.3mf".into()]);
        args.extend(transform.args());
        args.extend(profile_args);
        args.extend(id_args);
        args.extend(["--allow-newer-file".into(), "--outputdir".into(), out_dir.to_string_lossy().into_owned()]);
        if inputs.is_empty() {
            args.push(input.to_string_lossy().into_owned());
        } else {
            args.extend(inputs);
        }

        let mut run = self.run(&args, req.work_dir, &out_dir, req.progress).await?;
        // Projects from Bambu Studio can carry values OrcaSlicer's range
        // checks refuse (e.g. -1 for "automatic"). Drop those settings so
        // OrcaSlicer uses its defaults, and slice once more.
        let mut adjusted = Vec::new();
        if run.error.is_some() && is_3mf(&input) {
            let keys = crate::threemf::rejected_settings(&run.text);
            if !keys.is_empty() {
                let fixed = req.work_dir.join("input-orca.3mf");
                let (src, dest, k) = (input.clone(), fixed.clone(), keys.clone());
                tokio::task::spawn_blocking(move || crate::threemf::edit_settings(&src, &dest, &k, &[])).await??;
                *args.last_mut().unwrap() = fixed.to_string_lossy().into_owned();
                reset_dir(&out_dir).await?;
                tracing::info!("retrying without settings OrcaSlicer refused: {}", keys.join(", "));
                run = self.run(&args, req.work_dir, &out_dir, req.progress).await?;
                adjusted = keys;
            }
        }

        let Run { status, text, log_tail, error, output } = run;
        match (status.success(), output, error) {
            (true, Some(output), None) => Ok(SliceOutput {
                output,
                log_tail,
                adjusted,
                notes,
                plate: (!req.extras.is_empty()).then_some(plate),
                extra_objects,
            }),
            // With everything arranged by OrcaSlicer, this means the plate overflowed.
            (_, _, Some(err)) if !req.extras.is_empty() && err.contains("unprintable area") => {
                bail!("The models don't all fit on the plate. Remove a model or lower the copies, and slice again.")
            }
            (_, _, Some(_)) if refused_filament(&text).is_some() => {
                bail!("{} Choose another build plate.", refused_filament(&text).unwrap_or_default())
            }
            (_, _, Some(err)) => bail!("OrcaSlicer: {err}"),
            (_, None, None) => bail!(
                "OrcaSlicer exited ({}) without producing a file. Last output: {}",
                status,
                log_tail.last().cloned().unwrap_or_default()
            ),
            (false, Some(_), None) => bail!("OrcaSlicer exited with {status}"),
        }
    }

    /// One STL per object of the file (`obj_<number>_…`), by number.
    async fn export_stl(&self, input: &Path, dir: &Path, work_dir: &Path, settings: &[String], shape: &[String]) -> anyhow::Result<Vec<(usize, PathBuf)>> {
        reset_dir(dir).await?;
        let mut args = vec!["--export-stl".to_string()];
        args.extend(shape.iter().cloned());
        args.extend(settings.iter().cloned());
        args.extend(["--allow-newer-file".into(), "--outputdir".into(), dir.to_string_lossy().into_owned()]);
        args.push(input.to_string_lossy().into_owned());
        let run = self.run(&args, work_dir, dir, None).await?;
        let mut files = Vec::new();
        if let Ok(mut list) = tokio::fs::read_dir(dir.join("stl")).await {
            while let Some(f) = list.next_entry().await? {
                let name = f.file_name().to_string_lossy().into_owned();
                if let Some(n) = stl_number(&name) {
                    files.push((n, f.path()));
                }
            }
        }
        files.sort();
        if !run.status.success() || run.error.is_some() || files.is_empty() {
            bail!("{}", run.error.or_else(|| run.log_tail.last().cloned()).unwrap_or_else(|| format!("OrcaSlicer exited with {}", run.status)));
        }
        Ok(files)
    }

    /// The added models as meshes. A project becomes its plate's objects, which
    /// leaves its colours and settings behind.
    async fn extra_meshes(&self, req: &SliceRequest<'_>, settings: &[String]) -> anyhow::Result<Vec<ExtraMesh>> {
        let work = req.work_dir.join("extras-work");
        reset_dir(&work).await?;
        let mut out = Vec::new();
        for (i, extra) in req.extras.iter().enumerate() {
            let name = extra.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let shape: Vec<String> = if extra.orient { vec!["--orient".into(), "1".into()] } else { Vec::new() };
            let files = self
                .export_stl(&extra.path, &work.join(format!("export-{i}")), req.work_dir, settings, &shape)
                .await
                .with_context(|| format!("OrcaSlicer couldn't read the added model {name}"))?;
            let numbers = is_3mf(&extra.path)
                .then(|| crate::threemf::plate_object_numbers(&extra.path, extra.plate))
                .flatten();
            for (n, path) in files {
                if numbers.as_ref().is_some_and(|keep| !keep.contains(&n)) {
                    continue;
                }
                let named = work.join(extra_file_name(i, n));
                tokio::fs::rename(&path, &named).await?;
                out.push(ExtraMesh { path: named, copies: extra.copies.max(1), slot: extra.slot });
            }
        }
        if out.is_empty() {
            bail!("The added models have nothing to print on the chosen plate.");
        }
        Ok(out)
    }

    /// One plate's project with the added meshes arranged on it, each object
    /// on its filament.
    async fn combine(&self, req: &SliceRequest<'_>, input: &Path, meshes: &[ExtraMesh], settings: &[String], work: &Path) -> anyhow::Result<PathBuf> {
        let slots = crate::threemf::filament_count(input).unwrap_or(0);
        let extra_slots = req.filaments.len().saturating_sub(slots);
        let prepared = work.join("base.3mf");
        let (src, dest, plate) = (input.to_path_buf(), prepared.clone(), req.plate);
        tokio::task::spawn_blocking(move || crate::threemf::single_plate(&src, &dest, plate, extra_slots)).await??;

        let dir = work.join("combined");
        let mut project = prepared;
        let mut output = None;
        // Project settings OrcaSlicer refuses stop it here as well as when slicing.
        for attempt in 0..2 {
            reset_dir(&dir).await?;
            let mut args = vec!["--export-3mf".to_string(), "combined.3mf".into(), "--arrange".into(), "1".into()];
            args.extend(settings.iter().cloned());
            args.extend(["--allow-newer-file".into(), "--outputdir".into(), dir.to_string_lossy().into_owned()]);
            args.push(project.to_string_lossy().into_owned());
            for m in meshes {
                for _ in 0..m.copies {
                    args.push(m.path.to_string_lossy().into_owned());
                }
            }
            let run = self.run(&args, req.work_dir, &dir, None).await?;
            let keys = crate::threemf::rejected_settings(&run.text);
            if run.error.is_some() && attempt == 0 && !keys.is_empty() {
                let fixed = work.join("base-orca.3mf");
                let (src, dest) = (project.clone(), fixed.clone());
                tokio::task::spawn_blocking(move || crate::threemf::edit_settings(&src, &dest, &keys, &[])).await??;
                project = fixed;
                continue;
            }
            let file = dir.join("combined.3mf");
            if !run.status.success() || run.error.is_some() || !file.exists() {
                bail!(
                    "OrcaSlicer couldn't put the models together: {}",
                    run.error.or_else(|| run.log_tail.last().cloned()).unwrap_or_else(|| format!("it exited with {}", run.status))
                );
            }
            output = Some(file);
            break;
        }
        let combined = output.ok_or_else(|| anyhow!("OrcaSlicer couldn't put the models together"))?;
        let edited = work.join("combined-slots.3mf");
        let slot_by_extra: Vec<u32> = req.extras.iter().map(|e| e.slot).collect();
        let dest = edited.clone();
        tokio::task::spawn_blocking(move || {
            crate::threemf::set_object_slots(&combined, &dest, |name| extra_of_file(name).and_then(|i| slot_by_extra.get(i).copied()))
        })
        .await??;
        Ok(edited)
    }

    async fn run(
        &self,
        args: &[String],
        work_dir: &Path,
        out_dir: &Path,
        progress: Option<&(dyn Fn(SliceProgress) + Send + Sync)>,
    ) -> anyhow::Result<Run> {
        // OrcaSlicer writes progress as JSON lines to the --pipe path; a
        // plain file works, so read it while the slicer runs.
        let pipe = work_dir.join("progress.jsonl");
        let mut args = args.to_vec();
        if progress.is_some() {
            tokio::fs::write(&pipe, b"").await?;
            let at = args.len().saturating_sub(1);
            args.splice(at..at, ["--pipe".to_string(), pipe.to_string_lossy().into_owned()]);
        }
        let home = self.cfg.data_dir.join("orca-home");
        // GTK wants a private runtime dir, or OrcaSlicer's last output line is a warning about it.
        let runtime = home.join("run");
        tokio::fs::create_dir_all(&runtime).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).await?;
        }
        tracing::info!("slicing: {} {}", self.cfg.orca_path.display(), args.join(" "));
        let mut child = Command::new(&self.cfg.orca_path)
            .args(&args)
            .current_dir(work_dir)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("could not start OrcaSlicer at {}", self.cfg.orca_path.display()))?;

        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let read_out = tokio::spawn(async move {
            let mut s = Vec::new();
            let _ = stdout.read_to_end(&mut s).await;
            s
        });
        let read_err = tokio::spawn(async move {
            let mut s = Vec::new();
            let _ = stderr.read_to_end(&mut s).await;
            s
        });
        let deadline = tokio::time::sleep(self.cfg.slice_timeout);
        tokio::pin!(deadline);
        let mut every = tokio::time::interval(Duration::from_millis(500));
        let mut last = None;
        let status = loop {
            tokio::select! {
                s = child.wait() => break s?,
                _ = &mut deadline => {
                    let _ = child.kill().await;
                    bail!("slicing took longer than {} s and was stopped", self.cfg.slice_timeout.as_secs());
                }
                _ = every.tick(), if progress.is_some() => {
                    let text = tokio::fs::read_to_string(&pipe).await.unwrap_or_default();
                    if let (Some(report), Some(p)) = (latest_progress(&text), progress)
                        && last.as_ref() != Some(&report) {
                            p(report.clone());
                            last = Some(report);
                        }
                }
            }
        };
        let mut text = String::from_utf8_lossy(&read_out.await.unwrap_or_default()).into_owned();
        text.push_str(&String::from_utf8_lossy(&read_err.await.unwrap_or_default()));
        let log_tail: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(25)
            .map(|l| l.chars().take(300).collect())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // OrcaSlicer writes result.json with a return code; its error string
        // is "Success." when the code is 0 (2.4).
        let error = tokio::fs::read(out_dir.join("result.json"))
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| result_error(&v));

        let output = crate::threemf::find_output(out_dir);
        Ok(Run { status, text, log_tail, error, output })
    }
}

fn result_error(result: &Value) -> Option<String> {
    let code = result.get("return_code").and_then(|c| c.as_i64()).unwrap_or(0);
    let msg = result.get("error_string").and_then(|e| e.as_str()).unwrap_or("").trim();
    (code != 0).then(|| if msg.is_empty() { format!("return code {code}") } else { msg.to_string() })
}

/// An added model's object as a mesh file.
struct ExtraMesh {
    path: PathBuf,
    copies: u32,
    slot: u32,
}

/// The number in `obj_12_name.stl`.
fn stl_number(name: &str) -> Option<usize> {
    name.strip_prefix("obj_")?.split('_').next()?.parse().ok()
}

/// One OrcaSlicer run.
struct Run {
    status: std::process::ExitStatus,
    text: String,
    log_tail: Vec<String>,
    error: Option<String>,
    output: Option<PathBuf>,
}

/// Size, turn, copies and placement, as OrcaSlicer arguments.
#[derive(Debug, Clone, Copy, Default)]
struct Transform {
    scale: Option<f64>,
    rotate_z: Option<f64>,
    copies: u32,
    arrange: bool,
    orient: bool,
}

impl Transform {
    fn of(req: &SliceRequest<'_>) -> Self {
        Self { scale: req.scale, rotate_z: req.rotate_z, copies: req.copies, arrange: req.arrange, orient: req.orient }
    }

    /// Size and turn only, to make a mesh ready to go with other models.
    fn shape_args(&self) -> Vec<String> {
        let mut args = Transform { copies: 1, arrange: false, ..*self }.args();
        if self.orient {
            args.extend(["--orient".to_string(), "1".to_string()]);
        }
        args
    }

    /// OrcaSlicer applies transforms in the order given: scale and rotate the
    /// model first, then repeat it, then orient and arrange the result.
    fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(s) = self.scale {
            args.extend(["--scale".to_string(), format!("{s:.4}")]);
        }
        if let Some(r) = self.rotate_z {
            args.extend(["--rotate".to_string(), format!("{r:.2}")]);
        }
        if self.copies > 1 {
            args.extend(["--repetitions".to_string(), self.copies.to_string()]);
        }
        if self.arrange {
            args.extend(["--orient".to_string(), if self.orient { "1" } else { "0" }.to_string()]);
            args.extend(["--arrange".to_string(), "1".to_string()]);
        }
        args
    }
}

const BED_TYPE: &str = "curr_bed_type";
/// OrcaSlicer's plate when nothing says otherwise.
const DEFAULT_PLATE: &str = "Cool Plate";
/// The last complete progress line OrcaSlicer wrote, e.g.
/// `{"message":"Generating G-code","total_percent":75,...}`.
fn latest_progress(text: &str) -> Option<SliceProgress> {
    text.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l.trim()).ok()?;
        Some(SliceProgress {
            percent: v.get("total_percent")?.as_u64()?.min(100) as u8,
            message: v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string(),
        })
    })
}

/// "Plate 1: Cool Plate does not support filament 2" → plain words.
fn refused_filament(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.contains(" does not support filament "))?;
    let (plate, filament) = line.rsplit_once(" does not support filament ")?;
    let plate = plate.rsplit_once(": ").map(|(_, p)| p).unwrap_or(plate).trim();
    Some(format!("Filament {} can't go on the {}.", filament.trim(), plate_label(plate)))
}

fn plate_label(value: &str) -> &str {
    build_plate(value).map(|p| p.label).unwrap_or(value)
}

fn is_3mf(path: &Path) -> bool {
    path.extension().map(|e| e.eq_ignore_ascii_case("3mf")).unwrap_or(false)
}

async fn reset_dir(dir: &Path) -> anyhow::Result<()> {
    let _ = tokio::fs::remove_dir_all(dir).await;
    tokio::fs::create_dir_all(dir).await?;
    Ok(())
}

fn json_setting(bytes: &[u8], key: &str) -> Option<String> {
    first_string(serde_json::from_slice::<Value>(bytes).ok()?.get(key))
}

async fn set_json_setting(path: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let mut v: Value = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    if let Some(map) = v.as_object_mut() {
        map.insert(key.into(), Value::String(value.into()));
    }
    tokio::fs::write(path, serde_json::to_vec_pretty(&v)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_result_json() {
        assert_eq!(result_error(&json!({ "return_code": 0, "error_string": "Success." })), None);
        assert_eq!(result_error(&json!({ "return_code": 0 })), None);
        assert_eq!(
            result_error(&json!({ "return_code": -18, "error_string": "Invalid parameter value(s) included in the 3mf file." })),
            Some("Invalid parameter value(s) included in the 3mf file.".into())
        );
        assert_eq!(result_error(&json!({ "return_code": -3 })), Some("return code -3".into()));
    }

    #[test]
    fn picks_a_plate_the_filaments_take() {
        let fil = |cool: &str, pei: &str| json!({ "cool_plate_temp": [cool], "textured_plate_temp": [pei], "eng_plate_temp": ["70"] });
        let index = ProfileIndex {
            filaments: HashMap::from([("PLA".to_string(), fil("35", "55")), ("PETG".to_string(), fil("0", "70")), ("Odd".to_string(), fil("0", "0"))]),
            ..Default::default()
        };
        assert_eq!(index.plate_for("Cool Plate", &["PLA"], &[]), None);
        assert_eq!(index.plate_for("Cool Plate", &["PLA", "PETG"], &[]), Some("Textured PEI Plate"));
        assert_eq!(index.plate_for("Textured PEI Plate", &["PETG"], &[]), None);
        assert_eq!(index.plate_for("Cool Plate", &["Odd"], &[]), Some("Supertack Plate"));
        // Only plates the printer has.
        let owned = vec!["Cool Plate".to_string(), "Engineering Plate".to_string()];
        assert_eq!(index.plate_for("Cool Plate", &["PETG"], &owned), Some("Engineering Plate"));
        assert!(!index.takes("Cool Plate", &["PLA", "PETG"]));
        assert_eq!(index.refused_by("cool plate", &["PLA", "PETG"]), Some("PETG"));
        // Plates AirSlice doesn't know are left alone.
        assert!(index.takes("Smooth Glass", &["PETG"]));
        assert_eq!(build_plate("SuperTack Plate").map(|p| p.value), Some("Supertack Plate"));
    }

    #[test]
    fn reads_progress() {
        let text = "{\"message\":\"Generating infill toolpath\",\"plate_count\":1,\"plate_index\":1,\"plate_percent\":35,\"total_percent\":34}\n\
            {\"message\":\"Generating G-code\",\"plate_count\":1,\"plate_index\":1,\"plate_percent\":80,\"total_percent\":75}\n{\"message\":\"Expor";
        assert_eq!(latest_progress(text), Some(SliceProgress { percent: 75, message: "Generating G-code".into() }));
        assert_eq!(latest_progress(""), None);
    }

    #[test]
    fn explains_refused_filament() {
        let out = "best: 0 0 1\nPlate 1: Cool Plate does not support filament 2\nrun found error, return -61, exit...";
        assert_eq!(refused_filament(out).as_deref(), Some("Filament 2 can't go on the Cool Plate (smooth)."));
        assert_eq!(refused_filament("all good"), None);
    }

    #[test]
    fn extra_files_keep_their_model() {
        assert_eq!(extra_of_file(&extra_file_name(3, 12)), Some(3));
        assert_eq!(extra_of_file("t-rex2.stl"), None);
        assert_eq!(stl_number("obj_12_t-rex2.stl"), Some(12));
        assert_eq!(stl_number("notes.txt"), None);
    }

    #[test]
    fn transform_arguments() {
        let base = SliceRequest {
            input: Path::new("in.stl"),
            work_dir: Path::new("/tmp"),
            plate: 1,
            machine: None,
            process: None,
            bed_type: None,
            overrides: &[],
            plates: &[],
            progress: None,
            filaments: &[],
            arrange: true,
            orient: false,
            scale: Some(1.5),
            rotate_z: Some(90.0),
            copies: 3,
            extras: &[],
        };
        assert_eq!(
            Transform::of(&base).args().join(" "),
            "--scale 1.5000 --rotate 90.00 --repetitions 3 --orient 0 --arrange 1"
        );
        let plain = SliceRequest { arrange: false, scale: None, rotate_z: None, copies: 1, ..base };
        assert!(Transform::of(&plain).args().is_empty());
    }

    fn write(p: &Path, v: Value) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::to_vec(&v).unwrap()).unwrap();
    }

    #[test]
    fn loads_and_resolves_inheritance() {
        let dir = std::env::temp_dir().join(format!("as-prof-{}", uuid::Uuid::new_v4()));
        write(&dir.join("BBL.json"), json!({
            "machine_list": [
                {"name": "fdm_bbl_3dp_001_common", "sub_path": "machine/common.json"},
                {"name": "Bambu Lab P1S 0.4 nozzle", "sub_path": "machine/p1s.json"}
            ],
            "process_list": [
                {"name": "fdm_process_common", "sub_path": "process/common.json"},
                {"name": "0.20mm Standard @BBL X1C", "sub_path": "process/std.json"},
                {"name": "0.08mm Extra Fine @BBL A1M", "sub_path": "process/a1m.json"}
            ],
            "filament_list": [
                {"name": "Bambu PLA Basic @BBL X1C", "sub_path": "filament/pla.json"}
            ]
        }));
        write(&dir.join("BBL/machine/common.json"), json!({"name": "fdm_bbl_3dp_001_common", "instantiation": "false", "gcode_flavor": "marlin", "nozzle_diameter": ["0.4"]}));
        write(&dir.join("BBL/machine/p1s.json"), json!({"name": "Bambu Lab P1S 0.4 nozzle", "inherits": "fdm_bbl_3dp_001_common", "instantiation": "true", "printer_model": "Bambu Lab P1S", "default_print_profile": "0.20mm Standard @BBL X1C", "default_filament_profile": ["Bambu PLA Basic @BBL X1C"]}));
        write(&dir.join("BBL/process/common.json"), json!({"name": "fdm_process_common", "instantiation": "false", "layer_height": "0.2"}));
        write(&dir.join("BBL/process/std.json"), json!({"name": "0.20mm Standard @BBL X1C", "inherits": "fdm_process_common", "instantiation": "true", "compatible_printers": ["Bambu Lab P1S 0.4 nozzle"]}));
        write(&dir.join("BBL/process/a1m.json"), json!({"name": "0.08mm Extra Fine @BBL A1M", "instantiation": "true", "compatible_printers": ["Bambu Lab A1 mini 0.4 nozzle"]}));
        write(&dir.join("BBL/filament/pla.json"), json!({"name": "Bambu PLA Basic @BBL X1C", "inherits": "Generic PLA @System", "instantiation": "true", "filament_id": "GFA00", "compatible_printers": ["Bambu Lab P1S 0.4 nozzle"]}));
        write(&dir.join("OrcaFilamentLibrary.json"), json!({"filament_list": [{"name": "Generic PLA @System", "sub_path": "filament/gpla.json"}]}));
        write(&dir.join("OrcaFilamentLibrary/filament/gpla.json"), json!({"name": "Generic PLA @System", "filament_type": ["PLA"], "instantiation": "false"}));

        let idx = ProfileIndex::load(&dir).unwrap();
        let machines = idx.machines();
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].printer_model, "Bambu Lab P1S");
        assert_eq!(machines[0].nozzle, "0.4");
        assert_eq!(idx.machines["Bambu Lab P1S 0.4 nozzle"]["gcode_flavor"], "marlin");
        let c = idx.choices_for("Bambu Lab P1S 0.4 nozzle");
        assert_eq!(c.processes.len(), 1);
        assert_eq!(c.default_process.as_deref(), Some("0.20mm Standard @BBL X1C"));
        assert_eq!(idx.processes["0.20mm Standard @BBL X1C"]["layer_height"], "0.2");
        assert_eq!(c.filaments[0].filament_type.as_deref(), Some("PLA"), "inherited from the filament library");
        assert_eq!(idx.default_filament("Bambu Lab P1S 0.4 nozzle").as_deref(), Some("Bambu PLA Basic @BBL X1C"));
        let m = "Bambu Lab P1S 0.4 nozzle";
        assert_eq!(idx.filament_for_tray(m, Some("GFA00"), "PLA").as_deref(), Some("Bambu PLA Basic @BBL X1C"), "RFID id");
        assert_eq!(idx.filament_for_tray(m, None, "pla").as_deref(), Some("Bambu PLA Basic @BBL X1C"), "by material");
        assert_eq!(idx.filament_for_tray(m, None, "TPU"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
