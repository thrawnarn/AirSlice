//! Persisted data types.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- auth

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// sha256 of the cookie value; the raw token is never stored.
    pub hash: String,
    pub created_at: i64,
    pub last_seen: i64,
    #[serde(default)]
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// First characters of the key so the user can tell keys apart.
    pub prefix: String,
    /// sha256 of the full key.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub hash: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BambuAccount {
    pub token: Option<String>,
    pub email: Option<String>,
    /// `global` (api.bambulab.com) or `china` (api.bambulab.cn).
    #[serde(default = "default_region")]
    pub region: String,
    pub updated_at: Option<i64>,
}

fn default_region() -> String {
    "global".into()
}

// ---------------------------------------------------------------- printers

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FtpDataMode {
    /// Encrypted data channel with TLS session reuse (X1, P1, P2, H2) and a
    /// plain data channel for A1-series; falls back to the other on failure.
    #[default]
    Auto,
    /// Always encrypt the data channel (PROT P).
    Private,
    /// Never encrypt the data channel (PROT C) — needed by A1 / A1 mini.
    Clear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Printer {
    pub id: String,
    pub name: String,
    /// IP address or host name on the LAN.
    pub host: String,
    pub serial: String,
    /// LAN access code shown on the printer screen. Never sent to clients.
    #[serde(default)]
    pub access_code: String,
    /// OrcaSlicer printer model, e.g. "Bambu Lab P1S".
    pub printer_model: String,
    /// OrcaSlicer machine profile used when slicing for this printer,
    /// e.g. "Bambu Lab P1S 0.4 nozzle".
    pub machine_profile: String,
    /// Default process profile, e.g. "0.20mm Standard @BBL X1C".
    #[serde(default)]
    pub process_profile: Option<String>,
    /// Default filament profile for single-material models (STL/OBJ).
    #[serde(default)]
    pub filament_profile: Option<String>,
    #[serde(default)]
    pub ftp_data_mode: FtpDataMode,
    /// Build plates the owner has for this printer (OrcaSlicer `curr_bed_type`
    /// values). Empty: not set, any plate may be chosen.
    #[serde(default)]
    pub plates: Vec<String>,
    /// The plate usually on the printer; slices use it unless another is chosen.
    #[serde(default)]
    pub default_plate: Option<String>,
    /// Camera: "" (none), "printer" (P1/A1 port 6000) or "ha" (a Home
    /// Assistant camera entity, `camera_entity`).
    #[serde(default)]
    pub camera: String,
    #[serde(default)]
    pub camera_entity: Option<String>,
    /// The printer's power plug: a Home Assistant switch, and the power (W)
    /// and energy (kWh) sensors found on its device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_switch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_sensor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_sensor: Option<String>,
    /// Turn the plug off this long after a print ended and the nozzle cooled; 0 = never.
    #[serde(default)]
    pub power_off_after_min: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: i64,
}

fn yes() -> bool {
    true
}

impl Printer {
    pub fn is_a1_family(&self) -> bool {
        let m = self.printer_model.to_lowercase();
        m.ends_with(" a1") || m.ends_with(" a1 mini")
    }
}

// ---------------------------------------------------------------- jobs

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Makerworld,
    Printables,
    Thingiverse,
    /// File uploaded from the phone/browser.
    Upload,
    /// Not sent by AirSlice: started on the printer's screen, from a slicer
    /// on a computer or by another app, or imported from an older log. The
    /// print is in the history, but AirSlice has no file to print it again.
    External,
}

/// A file (or MakerWorld print profile) the user can pick from a model page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    /// Source-specific id (MakerWorld profileId, Printables file id).
    pub id: String,
    pub name: String,
    /// `profile` (MakerWorld 3MF print profile), `stl`, `3mf`, `obj`, `step`.
    pub kind: String,
    pub size_bytes: Option<u64>,
    pub thumbnail: Option<String>,
    /// Printer the profile was made for, when the source tells us.
    pub printer: Option<String>,
    /// Printers the source says this profile is compatible with.
    #[serde(default)]
    pub compatible: Vec<String>,
    pub plates: Option<u32>,
    pub need_ams: Option<bool>,
    #[serde(default)]
    pub filaments: Vec<FilamentInfo>,
    /// Printables download type (stl/gcode/sla/other).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_type: Option<String>,
    /// Folder on the source, if any (Printables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// What the source says the profile takes: print time and filament.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_g: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedModel {
    pub source: Source,
    /// Numeric id on the source site.
    pub model_id: String,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub cover: Option<String>,
    pub files: Vec<ModelFile>,
    /// File the link pointed at directly (MakerWorld `#profileId-…`).
    pub preselected_file_id: Option<String>,
    /// Hosts the model only offers files we cannot slice.
    #[serde(default)]
    pub notes: Vec<String>,
    /// MakerWorld internal alphanumeric model id, needed to download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_id: Option<String>,
    /// More about the model, for its page (not every source has it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ModelDetails>,
}

/// What a model page shows besides the files. The description is plain text
/// blocks, never the site's HTML (the app shares Home Assistant's origin).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelDetails {
    pub summary: Option<String>,
    pub description: Vec<TextBlock>,
    /// Pictures, cover first.
    pub images: Vec<String>,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub likes: Option<u64>,
    pub downloads: Option<u64>,
    pub prints: Option<u64>,
    pub collections: Option<u64>,
    pub license: Option<String>,
    /// Unix seconds.
    pub published_at: Option<i64>,
}

/// One block of a description: `p` paragraph, `h` heading, `li` list item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilamentInfo {
    #[serde(rename = "type")]
    pub kind: String,
    /// `#RRGGBB`
    pub color: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Downloading,
    /// File downloaded and inspected; waiting for plate/printer choice.
    Ready,
    Slicing,
    /// Sliced; waiting for filament mapping and the go.
    Sliced,
    Uploading,
    Starting,
    /// Waiting in a printer's queue for the plate to be cleared.
    Queued,
    /// Print command accepted by the printer.
    Started,
    Failed,
    /// Sent while the printer was offline: goes out when it is back.
    Waiting,
    /// The printer finished the print (see `print.outcome`).
    Finished,
}

impl JobState {
    /// AirSlice is working on it (a restart interrupts these).
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Downloading | Self::Slicing | Self::Uploading | Self::Starting | Self::Waiting)
    }

    /// Busy, or on the printer now: not to be re-sliced, re-queued or dropped
    /// from the history, or the print's outcome is lost.
    pub fn in_use(self) -> bool {
        self.is_busy() || self == Self::Started
    }
}

/// Spoolman (optional): spools per tray, what is left, prices. Kept in the
/// state file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpoolmanSettings {
    /// e.g. `http://spoolman.local:7912`; empty = not used.
    pub url: String,
    /// Book the filament a print used on its spools. Off by default: tools
    /// like HaspelSync or the Bambu integration often do that already.
    pub book_usage: bool,
    /// Trays linked to a spool by hand ("printer id:tray id" -> spool id),
    /// for spools without an RFID tag.
    pub links: std::collections::BTreeMap<String, i64>,
}

/// How long job files (downloads, sliced files) are kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageSettings {
    /// Remove the files of jobs not touched for this many days (0 = no
    /// limit by age). Favourites, queued and running jobs are always kept.
    pub keep_files_days: u32,
    /// Keep files for at most this many jobs, newest first (0 = no limit
    /// by number). Either limit removes files.
    pub keep_files_count: u32,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self { keep_files_days: 30, keep_files_count: 0 }
    }
}

/// Print cost estimates (optional). Without prices no cost is shown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostSettings {
    /// Shown after amounts, e.g. "kr" or "€".
    pub currency: String,
    /// Price of 1 kg of filament, for spools without a price in Spoolman.
    pub price_per_kg: Option<f64>,
    /// Running cost per print hour (power, wear).
    pub per_hour: Option<f64>,
}

/// A print's cost: filament + machine time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// `None` when a filament's price isn't known.
    pub filament: Option<f64>,
    pub machine: Option<f64>,
    pub total: f64,
    pub currency: String,
    /// Estimate from the slice, or what the print actually took.
    pub actual: bool,
}

/// Push notifications: shared Home Assistant connection plus any number of
/// notifications, each with its own channel, recipient and events. Kept in
/// the state file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NotifySettings {
    pub enabled: bool,
    /// Home Assistant address, e.g. `http://homeassistant.local:8123`. Empty
    /// in the add-on, which reaches Home Assistant through the Supervisor.
    pub ha_url: String,
    /// Long-lived access token (not needed in the add-on). Never sent back
    /// to the browser.
    pub token: String,
    /// Link opened when a notification is tapped; empty = the AirSlice
    /// panel in the add-on, nothing otherwise.
    pub open_url: String,
    pub targets: Vec<NotifyTarget>,
    /// Pre-alpha builds had one notification, stored in these fields;
    /// `migrate` turns it into a target. Read, never written.
    #[serde(skip_serializing)]
    channel: String,
    #[serde(skip_serializing)]
    service: String,
    #[serde(skip_serializing)]
    telegram_token: String,
    #[serde(skip_serializing)]
    telegram_chat: String,
    #[serde(skip_serializing)]
    ntfy_url: String,
    #[serde(skip_serializing)]
    ntfy_token: String,
    #[serde(skip_serializing)]
    photos: Option<bool>,
    #[serde(skip_serializing)]
    sliced: Option<bool>,
    #[serde(skip_serializing)]
    printing: Option<bool>,
    #[serde(skip_serializing)]
    finished: Option<bool>,
    #[serde(skip_serializing)]
    problems: Option<bool>,
    #[serde(skip_serializing)]
    queue: Option<bool>,
    #[serde(skip_serializing)]
    first_layer: Option<bool>,
}

impl NotifySettings {
    /// Move a single pre-alpha notification into `targets`.
    pub fn migrate(&mut self) {
        let configured = !self.service.trim().is_empty() || !self.telegram_chat.trim().is_empty() || !self.ntfy_url.trim().is_empty();
        if self.targets.is_empty() && configured {
            let d = NotifyTarget::default();
            let channel = match self.channel.as_str() {
                "telegram" | "ntfy" => self.channel.clone(),
                _ => "ha".to_string(),
            };
            self.targets.push(NotifyTarget {
                id: "1".into(),
                name: match channel.as_str() {
                    "telegram" => "Telegram".into(),
                    "ntfy" => "ntfy".into(),
                    _ => self.service.trim_start_matches("mobile_app_").replace('_', " "),
                },
                enabled: true,
                channel,
                service: std::mem::take(&mut self.service),
                telegram_token: std::mem::take(&mut self.telegram_token),
                telegram_chat: std::mem::take(&mut self.telegram_chat),
                ntfy_url: std::mem::take(&mut self.ntfy_url),
                ntfy_token: std::mem::take(&mut self.ntfy_token),
                photos: self.photos.unwrap_or(d.photos),
                language: String::new(),
                sliced: self.sliced.unwrap_or(d.sliced),
                printing: self.printing.unwrap_or(d.printing),
                finished: self.finished.unwrap_or(d.finished),
                problems: self.problems.unwrap_or(d.problems),
                queue: self.queue.unwrap_or(d.queue),
                first_layer: self.first_layer.unwrap_or(d.first_layer),
                ..NotifyTarget::default()
            });
        }
    }
}

/// One notification: where it goes and which events it carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyTarget {
    pub id: String,
    /// What the owner calls it, e.g. "My phone".
    pub name: String,
    pub enabled: bool,
    /// "ha" (Home Assistant notify service), "telegram", "ntfy" or
    /// "webpush" (a browser that turned notifications on).
    pub channel: String,
    /// The notify service without `notify.`, e.g. `mobile_app_my_phone`.
    pub service: String,
    /// Telegram bot token (never sent back to the browser) and chat id.
    pub telegram_token: String,
    pub telegram_chat: String,
    /// ntfy topic URL, e.g. `https://ntfy.sh/my-printer`, and an optional
    /// access token (never sent back).
    pub ntfy_url: String,
    pub ntfy_token: String,
    /// Web Push: the browser's endpoint and keys (p256dh, auth; never sent
    /// back). Set only by the browser subscribing.
    pub push_endpoint: String,
    pub push_p256dh: String,
    pub push_auth: String,
    /// Add a camera picture when the printer has a camera.
    pub photos: bool,
    /// "en", "sv", or "" for Home Assistant's language.
    #[serde(default)]
    pub language: String,
    /// Which events notify.
    pub sliced: bool,
    pub printing: bool,
    pub finished: bool,
    pub problems: bool,
    pub queue: bool,
    /// After the first layer, so a bad start can be stopped early.
    pub first_layer: bool,
    /// The printer needs a person: paused, a serious message, contact lost.
    pub attention: bool,
    /// Five minutes before the end.
    pub almost_done: bool,
}

impl Default for NotifyTarget {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            channel: "ha".into(),
            service: String::new(),
            telegram_token: String::new(),
            telegram_chat: String::new(),
            ntfy_url: String::new(),
            ntfy_token: String::new(),
            push_endpoint: String::new(),
            push_p256dh: String::new(),
            push_auth: String::new(),
            photos: true,
            language: String::new(),
            sliced: true,
            printing: true,
            finished: true,
            problems: true,
            queue: true,
            first_layer: true,
            attention: true,
            almost_done: true,
        }
    }
}

/// How far a slice has come, as OrcaSlicer reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SliceProgress {
    pub percent: u8,
    /// OrcaSlicer's step, e.g. "Generating G-code".
    pub message: String,
}

/// How AirSlice deals with a printer that is offline or flaky. Kept in the
/// state file and edited under Settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionSettings {
    /// How long a print waits for an offline printer before giving up (0 =
    /// fail straight away).
    pub wait_minutes: u32,
    /// Upload tries before giving up.
    pub upload_attempts: u32,
    /// Pause before the second try; doubles after each failure.
    pub retry_delay_secs: u32,
    /// How long to wait for the printer to report the print as started.
    pub start_timeout_secs: u32,
    /// Longest pause between reconnect attempts while a printer is offline.
    pub reconnect_max_secs: u32,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self { wait_minutes: 30, upload_attempts: 3, retry_delay_secs: 10, start_timeout_secs: 30, reconnect_max_secs: 30 }
    }
}

impl ConnectionSettings {
    pub fn validate(&self) -> Result<(), String> {
        let check = |v: u32, lo: u32, hi: u32, what: &str| {
            if (lo..=hi).contains(&v) { Ok(()) } else { Err(format!("{what} must be between {lo} and {hi}")) }
        };
        check(self.wait_minutes, 0, 24 * 60, "waiting for an offline printer (minutes)")?;
        check(self.upload_attempts, 1, 10, "upload attempts")?;
        check(self.retry_delay_secs, 1, 600, "pause between attempts (seconds)")?;
        check(self.start_timeout_secs, 5, 600, "start confirmation (seconds)")?;
        check(self.reconnect_max_secs, 2, 600, "longest reconnect pause (seconds)")
    }

    /// Pause after failed attempt `n` (1-based): the delay, doubling, at most 10 minutes.
    pub fn retry_pause(&self, n: u32) -> std::time::Duration {
        let secs = (self.retry_delay_secs as u64).saturating_mul(1u64 << n.saturating_sub(1).min(16));
        std::time::Duration::from_secs(secs.min(600))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlateInfo {
    pub index: u32,
    pub name: Option<String>,
    pub has_thumbnail: bool,
    #[serde(default)]
    pub object_count: u32,
    /// The objects on the plate, in the project's order.
    #[serde(default)]
    pub objects: Vec<PlateObject>,
    /// Print time and filament from the file (when it was sliced before
    /// saving) or from AirSlice's own slice of this plate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_g: Option<f64>,
    /// Filament slots this plate uses; `None` when the file doesn't tell without
    /// reading its meshes (painted models, colour changes per layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filaments: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateObject {
    /// Object id in the 3MF.
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inspection {
    /// `project` (3MF), `sliced` (3MF with G-code), `mesh` (STL/OBJ/STEP).
    pub kind: String,
    pub plates: Vec<PlateInfo>,
    /// Printer model the project was set up for, e.g. "Bambu Lab A1".
    pub printer_model: Option<String>,
    pub printer_profile: Option<String>,
    pub filaments: Vec<FilamentInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlicedFilament {
    /// 1-based filament slot in the project.
    pub id: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub color: String,
    pub used_g: f64,
    pub used_m: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceOutcome {
    pub printer_id: String,
    pub plate: u32,
    /// Entry inside the 3MF, e.g. "Metadata/plate_2.gcode".
    pub gcode_entry: String,
    /// Total filament slots in the project (length of the AMS mapping).
    pub filament_slots: u32,
    pub filaments: Vec<SlicedFilament>,
    pub prediction_secs: Option<u64>,
    pub weight_g: Option<f64>,
    /// Which settings were used: `project`, `printer` or `presliced`.
    pub settings: String,
    /// Mesh adjustments applied while slicing, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<MeshTransform>,
    /// Filament profile used for a mesh, when chosen by AirSlice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filament_profile: Option<String>,
    /// Trays chosen before slicing (index = slot - 1), pre-selected for printing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_trays: Option<Vec<i32>>,
    /// Build plate the G-code was made for (`curr_bed_type` value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_type: Option<String>,
    /// Setting changes made for this slice.
    #[serde(default, skip_serializing_if = "SliceTweaks::is_empty")]
    pub tweaks: SliceTweaks,
    /// What was asked for, to slice the same again for another printer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SliceOptions>,
    /// The files on the plate, when models were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<SlicedFile>,
    #[serde(skip)]
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintStart {
    pub printer_id: String,
    pub remote_file: String,
    pub ams_mapping: Vec<i32>,
    pub use_ams: bool,
    pub started_at: i64,
    /// The printer reported this print as running at least once.
    #[serde(default)]
    pub seen_running: bool,
    /// How it ended, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PrintOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Printer error code when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i64>,
    /// How far it got (percent), for partial filament use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<i64>,
    /// The first layer is down (notified once).
    #[serde(default)]
    pub first_layer_done: bool,
    /// The last five minutes have been announced.
    #[serde(default)]
    pub almost_done_sent: bool,
    /// The plug's energy meter at the start (kWh), and what the print used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_kwh: Option<f64>,
    /// The last layer the printer reported, and how many there are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_layers: Option<i64>,
    /// Filament that went to colour changes rather than the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waste: Option<PrintWaste>,
}

impl PrintStart {
    /// The name the printer shows for this print (`subtask_name`).
    pub fn task_name(&self) -> &str {
        self.remote_file.trim_end_matches(".3mf").trim_end_matches(".gcode")
    }

    /// Is this what the printer reports as its current task?
    pub fn is_task(&self, reported: Option<&str>) -> bool {
        reported.is_some_and(|r| r.trim_end_matches(".3mf").trim_end_matches(".gcode") == self.task_name())
    }
}

/// Purge and prime tower of a multi-colour print, from the sliced G-code. Part
/// of the filament weight and cost already, not on top of them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrintWaste {
    pub changes: u32,
    pub flush_g: f64,
    pub tower_g: f64,
    /// What that filament cost, when its price is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrintOutcome {
    Finished,
    Failed,
    /// Stopped on the printer or from AirSlice.
    Cancelled,
    /// Another print started, or the printer forgot it: no result.
    Unknown,
}

/// Adjustments for bare meshes (STL/OBJ/STEP) before slicing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MeshTransform {
    /// Uniform scale in percent (100 = unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_percent: Option<f64>,
    /// Number of copies on the plate (1 = just the model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copies: Option<u32>,
    /// Rotation around the vertical axis in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate_z: Option<f64>,
    /// Let the slicer pick the best orientation (default on; turned off
    /// automatically when a rotation is given, since it would undo it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_orient: Option<bool>,
}

impl MeshTransform {
    pub fn scale_factor(&self) -> Option<f64> {
        self.scale_percent.filter(|p| (*p - 100.0).abs() > 0.001).map(|p| p / 100.0)
    }
    pub fn copies(&self) -> u32 {
        self.copies.unwrap_or(1).max(1)
    }
    pub fn rotation(&self) -> Option<f64> {
        self.rotate_z.map(|r| r.rem_euclid(360.0)).filter(|r| r.abs() > 0.001)
    }
    pub fn orient(&self) -> bool {
        self.auto_orient.unwrap_or(self.rotation().is_none())
    }
    /// Scale or rotation set; only meshes support those.
    pub fn changes_shape(&self) -> bool {
        self.scale_factor().is_some() || self.rotation().is_some() || self.auto_orient.is_some()
    }
    pub fn is_identity(&self) -> bool {
        self.scale_factor().is_none() && self.copies() == 1 && self.rotation().is_none() && self.auto_orient.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(p) = self.scale_percent
            && !(1.0..=1000.0).contains(&p) {
                return Err("scale must be between 1 % and 1000 %".into());
            }
        if let Some(c) = self.copies
            && !(1..=50).contains(&c) {
                return Err("copies must be between 1 and 50".into());
            }
        if let Some(r) = self.rotate_z
            && !r.is_finite() {
                return Err("rotation must be a number of degrees".into());
            }
        Ok(())
    }
    /// "150 %, 3 copies, rotated 90°"
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(f) = self.scale_factor() {
            parts.push(format!("{:.0} %", f * 100.0));
        }
        if self.copies() > 1 {
            parts.push(format!("{} copies", self.copies()));
        }
        if let Some(r) = self.rotation() {
            parts.push(format!("rotated {r:.0}°"));
        }
        parts.join(", ")
    }
}

/// A sliced job waiting for its printer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub job_id: String,
    pub printer_id: String,
    pub options: PrintOptions,
    pub added_at: i64,
}

/// Options the user picks before slicing. Also used by quick-print.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SliceOptions {
    pub printer_id: Option<String>,
    pub plate: Option<u32>,
    /// `auto` (project settings if made for this printer, else printer
    /// profiles), `project` or `printer`.
    #[serde(default)]
    pub settings: Option<String>,
    pub process_profile: Option<String>,
    pub filament_profile: Option<String>,
    /// Scale, copies and rotation; only for meshes.
    #[serde(default, flatten)]
    pub transform: MeshTransform,
    /// Meshes: slice with the filament loaded in this tray (global tray id,
    /// 254 = external spool) and pre-select it for printing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_id: Option<i32>,
    /// One tray per project filament (index = slot - 1, -1 = none chosen):
    /// slice with each tray's filament profile and colour, and pre-select
    /// the trays for printing. Pre-sliced files only use it for printing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_ids: Option<Vec<i32>>,
    /// Objects (3MF ids) to print from the plate; all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objects: Option<Vec<String>>,
    /// Build plate to slice for; the printer's default plate when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed_type: Option<String>,
    /// Changes to the profile's or project's settings for this slice.
    #[serde(default, skip_serializing_if = "SliceTweaks::is_empty")]
    pub tweaks: SliceTweaks,
    /// Models added to the plate (`Job::extras`) to print along with this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<ExtraChoice>,
    /// Other plates of this file to print together with `plate`, on one plate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_plates: Vec<u32>,
}

/// How one added model goes on the plate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtraChoice {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copies: Option<u32>,
    /// A 3MF's plate to take the objects from; its first when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate: Option<u32>,
    /// Tray to print it from (global tray id); the first filament when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_id: Option<i32>,
}

/// One file in a slice and the filament slots it prints with. Slots can be
/// shared between files that use the same tray.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlicedFile {
    /// The added model's id; `None` for the job's own file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    pub slots: Vec<u32>,
    pub copies: u32,
}

/// A model added to a job's plate, kept in the job's `extras/` folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlateExtra {
    pub id: String,
    pub title: String,
    pub source: Source,
    #[serde(default)]
    pub model_url: String,
    pub cover: Option<String>,
    /// File name inside the job's `extras/` folder.
    pub input_file: String,
    /// `mesh`, `project` or `sliced`, as in `Inspection::kind`.
    pub kind: String,
    #[serde(default)]
    pub plates: Vec<PlateInfo>,
    /// It has several filaments in the file, and prints in one here.
    #[serde(default)]
    pub multicolor: bool,
}

/// Quick changes to the print settings; unset fields keep the profile's or
/// project's own value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SliceTweaks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls: Option<u32>,
    /// Sparse infill, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infill: Option<u32>,
    /// "off", "normal" or "tree".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brim: Option<bool>,
}

impl SliceTweaks {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(h) = self.layer_height
            && !(0.04..=0.6).contains(&h)
        {
            return Err(format!("layer height {h} mm is outside 0.04–0.6 mm"));
        }
        if self.walls.is_some_and(|w| !(1..=20).contains(&w)) {
            return Err("walls must be between 1 and 20".into());
        }
        if self.infill.is_some_and(|i| i > 100) {
            return Err("infill must be 0–100 %".into());
        }
        if let Some(s) = &self.supports
            && !["off", "normal", "tree"].contains(&s.as_str())
        {
            return Err(format!("unknown support type {s:?}"));
        }
        Ok(())
    }

    /// OrcaSlicer setting keys and values (process / project settings).
    pub fn settings(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut set = |k: &str, v: String| out.push((k.to_string(), v));
        if let Some(h) = self.layer_height {
            set("layer_height", format!("{h}"));
        }
        if let Some(w) = self.walls {
            set("wall_loops", w.to_string());
        }
        if let Some(i) = self.infill {
            set("sparse_infill_density", format!("{i}%"));
        }
        match self.supports.as_deref() {
            Some("off") => set("enable_support", "0".into()),
            Some(kind @ ("normal" | "tree")) => {
                set("enable_support", "1".into());
                set("support_type", format!("{kind}(auto)"));
            }
            _ => {}
        }
        match self.brim {
            Some(true) => {
                set("brim_type", "outer_only".into());
                set("brim_width", "5".into());
            }
            Some(false) => set("brim_type", "no_brim".into()),
            None => {}
        }
        out
    }

    /// "0.28 mm layers, 4 walls, 40 % infill, tree supports, brim".
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(h) = self.layer_height {
            parts.push(format!("{h} mm layers"));
        }
        if let Some(w) = self.walls {
            parts.push(format!("{w} walls"));
        }
        if let Some(i) = self.infill {
            parts.push(format!("{i} % infill"));
        }
        match self.supports.as_deref() {
            Some("off") => parts.push("no supports".into()),
            Some(kind) => parts.push(format!("{kind} supports")),
            None => {}
        }
        match self.brim {
            Some(true) => parts.push("brim".into()),
            Some(false) => parts.push("no brim".into()),
            None => {}
        }
        parts.join(", ")
    }
}

/// Saved slice settings, picked with one tap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlicePreset {
    pub name: String,
    #[serde(flatten)]
    pub tweaks: SliceTweaks,
}

/// What a new installation offers.
pub fn default_presets() -> Vec<SlicePreset> {
    vec![
        SlicePreset { name: "Draft".into(), tweaks: SliceTweaks { layer_height: Some(0.28), infill: Some(10), ..Default::default() } },
        SlicePreset { name: "Strong".into(), tweaks: SliceTweaks { walls: Some(4), infill: Some(40), ..Default::default() } },
        SlicePreset { name: "Fine".into(), tweaks: SliceTweaks { layer_height: Some(0.12), ..Default::default() } },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintOptions {
    /// One entry per project filament slot: global AMS tray id
    /// (ams*4+slot), 254 for the external spool, -1 for unused.
    /// Omit to use the automatic mapping.
    #[serde(default)]
    pub ams_mapping: Option<Vec<i32>>,
    #[serde(default = "yes")]
    pub bed_leveling: bool,
    #[serde(default)]
    pub timelapse: bool,
    #[serde(default = "yes")]
    pub flow_cali: bool,
    #[serde(default = "yes")]
    pub vibration_cali: bool,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self { ams_mapping: None, bed_leveling: true, timelapse: false, flow_cali: true, vibration_cali: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub state: JobState,
    pub error: Option<String>,
    pub log: Vec<String>,
    pub source: Source,
    pub model_url: String,
    pub model_id: String,
    pub model_title: String,
    pub model_cover: Option<String>,
    pub file: ModelFile,
    pub inspection: Option<Inspection>,
    pub slice: Option<SliceOutcome>,
    pub print: Option<PrintStart>,
    /// Quick-print: run every step automatically.
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub auto_slice: Option<SliceOptions>,
    #[serde(default)]
    pub auto_print: Option<PrintOptions>,
    /// Downloaded file name inside the job directory.
    #[serde(default)]
    pub input_file: Option<String>,
    /// Sliced file name inside the job directory.
    #[serde(default)]
    pub output_file: Option<String>,
    /// Other models to print on the same plate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<PlateExtra>,
    /// Kept out of history clean-up and listed under Favourites.
    #[serde(default)]
    pub favorite: bool,
    /// Job this one was copied from by "Print again".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copied_from: Option<String>,
    /// Shown next to the title for one plate of a multi-plate model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_label: Option<String>,
    /// Queue automatically once sliced ("Queue all plates").
    #[serde(default)]
    pub auto_queue: bool,
    /// While waiting for an offline printer: when to give up (unix seconds)
    /// and what to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_print: Option<PrintOptions>,
    /// What the print cost, worked out when it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// The downloaded and sliced files were removed to save space.
    #[serde(default)]
    pub files_removed: bool,
    /// A camera picture was kept when the print ended.
    #[serde(default)]
    pub photo: bool,
}

impl Job {
    pub fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        tracing::info!(job = %self.id, "{line}");
        self.log.push(line);
        if self.log.len() > 200 {
            let excess = self.log.len() - 200;
            self.log.drain(0..excess);
        }
        self.updated_at = now();
    }

    pub fn fail(&mut self, err: impl Into<String>) {
        let err = err.into();
        self.push_log(format!("Failed: {err}"));
        self.error = Some(err);
        self.state = JobState::Failed;
    }
}

#[cfg(test)]
mod notify_tests {
    use super::*;

    #[test]
    fn single_notification_becomes_a_target() {
        let mut s: NotifySettings = serde_json::from_value(serde_json::json!({
            "enabled": true, "channel": "telegram", "telegram_token": "123:ABC", "telegram_chat": "42",
            "sliced": false, "photos": false
        }))
        .unwrap();
        s.migrate();
        assert_eq!(s.targets.len(), 1);
        let t = &s.targets[0];
        assert_eq!((t.channel.as_str(), t.telegram_chat.as_str(), t.telegram_token.as_str()), ("telegram", "42", "123:ABC"));
        assert!(!t.sliced && !t.photos && t.finished);
        // The old fields aren't written back.
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("telegram_token").is_none() && v.get("channel").is_none());
        // A second load with targets doesn't add another.
        s.migrate();
        assert_eq!(s.targets.len(), 1);
    }
}

#[cfg(test)]
mod tweak_tests {
    use super::*;

    #[test]
    fn tweaks_become_settings() {
        let t = SliceTweaks { layer_height: Some(0.28), walls: Some(4), infill: Some(40), supports: Some("tree".into()), brim: Some(false) };
        let s = t.settings();
        assert!(s.contains(&("layer_height".into(), "0.28".into())));
        assert!(s.contains(&("wall_loops".into(), "4".into())));
        assert!(s.contains(&("sparse_infill_density".into(), "40%".into())));
        assert!(s.contains(&("support_type".into(), "tree(auto)".into())));
        assert!(s.contains(&("brim_type".into(), "no_brim".into())));
        assert_eq!(t.describe(), "0.28 mm layers, 4 walls, 40 % infill, tree supports, no brim");
        assert!(SliceTweaks::default().settings().is_empty());
        assert!(SliceTweaks { layer_height: Some(1.0), ..Default::default() }.validate().is_err());
        assert!(SliceTweaks { supports: Some("x".into()), ..Default::default() }.validate().is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_transform_rules() {
        let t = MeshTransform::default();
        assert!(t.is_identity());
        assert!(t.orient());
        let t = MeshTransform { scale_percent: Some(100.0), copies: Some(1), ..Default::default() };
        assert!(t.is_identity(), "100 % and one copy change nothing");
        let t = MeshTransform { scale_percent: Some(150.0), copies: Some(3), rotate_z: Some(-90.0), auto_orient: None };
        assert_eq!(t.scale_factor(), Some(1.5));
        assert_eq!(t.rotation(), Some(270.0));
        assert!(!t.orient(), "a manual rotation turns auto-orient off");
        assert_eq!(t.describe(), "150 %, 3 copies, rotated 270°");
        assert!(MeshTransform { scale_percent: Some(0.5), ..Default::default() }.validate().is_err());
        assert!(MeshTransform { copies: Some(51), ..Default::default() }.validate().is_err());
        assert!(MeshTransform { rotate_z: Some(360.0), ..Default::default() }.rotation().is_none());
    }

    #[test]
    fn slice_options_accept_flat_transform_fields() {
        let o: SliceOptions = serde_json::from_str(r#"{"plate": 1, "scale_percent": 200, "copies": 2}"#).unwrap();
        assert_eq!(o.transform.scale_factor(), Some(2.0));
        assert_eq!(o.transform.copies(), 2);
    }
}
