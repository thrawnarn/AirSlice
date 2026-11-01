//! Interpreting the printer's MQTT `report` messages.

use serde::Serialize;
use serde_json::Value;

/// Global tray id used by the firmware for the external spool.
pub const EXTERNAL_TRAY: i32 = 254;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tray {
    /// Global id: ams_id * 4 + slot, an AMS HT's unit id, or 254 for the external spool.
    pub id: i32,
    pub ams: Option<u32>,
    pub slot: Option<u32>,
    /// e.g. "PLA", "PETG". Empty when the slot is empty.
    #[serde(rename = "type")]
    pub kind: String,
    /// `#RRGGBB`
    pub color: String,
    pub name: Option<String>,
    /// Remaining filament in percent, when known (-1 from firmware = unknown).
    pub remain: Option<i32>,
    /// Full spool weight in grams (`tray_weight`, from the RFID tag).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spool_g: Option<i32>,
    /// Bambu filament id from the spool's RFID tag (e.g. "GFA00" = PLA Basic).
    pub info_idx: Option<String>,
    /// The spool's RFID tag id (what Spoolman tools store as `extra.tag`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub active: bool,
    /// Drying values from the spool's RFID tag.
    pub dry_temp: Option<i32>,
    pub dry_hours: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StatusView {
    pub connected: bool,
    pub last_error: Option<String>,
    pub last_update: Option<i64>,
    /// IDLE, PREPARE, RUNNING, PAUSE, FINISH, FAILED (firmware gcode_state).
    pub state: Option<String>,
    pub task_name: Option<String>,
    pub percent: Option<i64>,
    pub remaining_minutes: Option<i64>,
    pub layer: Option<i64>,
    pub total_layers: Option<i64>,
    pub nozzle_temp: Option<f64>,
    pub nozzle_target: Option<f64>,
    pub bed_temp: Option<f64>,
    pub bed_target: Option<f64>,
    pub chamber_light: Option<bool>,
    /// Print speed level: 1 silent, 2 standard, 3 sport, 4 ludicrous.
    pub speed_level: Option<i64>,
    pub print_error: Option<i64>,
    /// Active HMS codes, e.g. "0500_0500_0001_0007".
    pub hms: Vec<String>,
    /// The same codes with what's known about them, for the printer card.
    pub problems: Vec<Problem>,
    pub has_ams: bool,
    pub trays: Vec<Tray>,
    pub external: Option<Tray>,
    /// AMS units with what they report about themselves.
    pub ams_units: Vec<AmsUnit>,
    /// Wi-Fi signal in dBm (e.g. -52).
    pub wifi_dbm: Option<i64>,
    /// Fan speeds in percent: part cooling, auxiliary, chamber.
    pub fan_part: Option<i64>,
    pub fan_aux: Option<i64>,
    pub fan_chamber: Option<i64>,
    /// e.g. "0.4".
    pub nozzle_diameter: Option<String>,
    /// For the sensors page.
    pub fan_heatbreak: Option<i64>,
    /// e.g. "hardened_steel".
    pub nozzle_type: Option<String>,
    /// Speed as a percentage of standard (`spd_mag`).
    pub speed_percent: Option<i64>,
    /// What the printer is doing within a print (`stg_cur`, e.g. 14 = cleaning the nozzle).
    pub stage: Option<i64>,
    /// "local", "cloud", "idle".
    pub print_type: Option<String>,
    pub gcode_file: Option<String>,
    pub sdcard: Option<bool>,
}

/// An AMS unit. Older units only report a humidity level (1 wet – 5 dry);
/// newer ones the relative humidity and temperature too.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AmsUnit {
    pub id: u32,
    /// "AMS A", "AMS HT 1".
    pub name: String,
    pub humidity_pct: Option<i64>,
    pub humidity_level: Option<i64>,
    pub temp: Option<f64>,
    /// Drying time left in minutes, while drying.
    pub dry_minutes: Option<i64>,
    /// "AMS 2 Pro", "AMS HT", "AMS", "AMS Lite", from the printer's module list.
    pub model: Option<String>,
    /// Highest drying temperature in °C; `None` when this unit can't dry.
    pub dry_max_temp: Option<u32>,
    /// Feeds the running print, so the printer won't dry it.
    pub in_print: bool,
}

/// Unit id and model from a `get_version` module name ("n3f/0", "n3s/128").
pub fn ams_module(name: &str) -> Option<(u32, &'static str)> {
    let (kind, id) = name.split_once('/')?;
    let model = match kind {
        "ams" => "AMS",
        "ams_f1" => "AMS Lite",
        "n3f" => "AMS 2 Pro",
        "n3s" => "AMS HT",
        _ => return None,
    };
    Some((id.parse().ok()?, model))
}

fn dry_max_temp(model: &str) -> Option<u32> {
    match model {
        "AMS 2 Pro" => Some(65),
        "AMS HT" => Some(85),
        _ => None,
    }
}

impl AmsUnit {
    /// Fill in what the status report doesn't say. Units from 128 up are always
    /// an AMS HT; below that it takes the module list.
    pub fn set_model(&mut self, models: &std::collections::HashMap<u32, String>) {
        self.model = models.get(&self.id).cloned().or_else(|| (self.id >= 128).then(|| "AMS HT".to_string()));
        self.dry_max_temp = self.model.as_deref().and_then(dry_max_temp);
    }
}

/// The AMS unit a global tray id belongs to; `None` for the external spool.
pub fn tray_unit(tray: i32) -> Option<u32> {
    match tray {
        EXTERNAL_TRAY => None,
        t if t < 0 => None,
        t if t >= 128 => Some(t as u32),
        t => Some(t as u32 / 4),
    }
}

/// An active HMS message.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Problem {
    pub code: String,
    /// fatal, serious, common or info (from the code's third group).
    pub severity: &'static str,
    /// What the printer says, in English; `texts` has it per language
    /// ("en", "sv") from Bambu's HMS list.
    pub text: Option<String>,
    pub texts: std::collections::HashMap<&'static str, String>,
    /// Bambu's wiki page for the code (its HMS overview when there is none).
    pub url: String,
}

impl Problem {
    pub fn new(code: &str) -> Self {
        let severity = match code.split('_').nth(2) {
            Some("0001") => "fatal",
            Some("0002") => "serious",
            Some("0003") => "common",
            _ => "info",
        };
        let mut texts = super::hms::texts(code);
        // AirSlice's own explanation says what to do about it.
        if let Some(own) = hms_explained(code) {
            texts.insert("en", own.to_string());
        }
        Problem { code: code.to_string(), severity, text: texts.get("en").cloned(), texts, url: super::hms::wiki_url(code) }
    }
}

impl Tray {
    /// "AMS A2", "AMS HT 1" or "the external spool".
    pub fn label(&self) -> String {
        match (self.id, self.ams, self.slot) {
            (EXTERNAL_TRAY, _, _) => "the external spool".into(),
            (_, Some(a), _) if a >= 128 => format!("AMS HT {}", a - 127),
            (_, Some(a), Some(s)) => format!("AMS {}{}", (b'A' + a as u8) as char, s + 1),
            _ => format!("tray {}", self.id + 1),
        }
    }
}

impl StatusView {
    pub fn is_busy(&self) -> bool {
        matches!(self.state.as_deref(), Some("RUNNING" | "PREPARE" | "PAUSE" | "SLICING"))
    }

    /// Trays that can feed a print (AMS slots with filament + external spool).
    pub fn usable_trays(&self) -> Vec<&Tray> {
        self.trays.iter().chain(self.external.iter()).filter(|t| !t.kind.is_empty()).collect()
    }
}

/// Deep-merge `patch` into `base`. P1/A1 printers send partial updates.
pub fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge(existing, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(num).map(|f| f as i64)
}

fn string(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.is_empty())
}

/// "FF8800FF" (RGBA) -> "#FF8800"
pub fn tray_color(raw: &str) -> String {
    let hex: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).take(6).collect();
    if hex.len() == 6 { format!("#{}", hex.to_uppercase()) } else { String::new() }
}

fn parse_tray(t: &Value, ams: Option<u32>, active_id: Option<i64>) -> Option<Tray> {
    let slot = int(t.get("id"))? as u32;
    let id = match ams {
        Some(a) if a >= 128 => a as i32, // AMS HT: one slot, id is the unit id
        Some(a) => (a * 4 + slot) as i32,
        None => slot as i32,
    };
    Some(Tray {
        id,
        ams,
        slot: ams.map(|_| slot),
        kind: string(t.get("tray_type")).unwrap_or_default(),
        color: string(t.get("tray_color")).map(|c| tray_color(&c)).unwrap_or_default(),
        name: string(t.get("tray_sub_brands")).or_else(|| string(t.get("tray_id_name"))),
        remain: int(t.get("remain")).map(|r| r as i32).filter(|r| *r >= 0),
        spool_g: int(t.get("tray_weight")).map(|w| w as i32).filter(|w| *w > 0),
        info_idx: string(t.get("tray_info_idx")),
        uuid: string(t.get("tray_uuid")).filter(|u| u.chars().any(|c| c != '0')),
        active: active_id == Some(id as i64),
        dry_temp: int(t.get("tray_temp")).map(|v| v as i32).filter(|v| *v > 0),
        dry_hours: int(t.get("tray_time")).map(|v| v as i32).filter(|v| *v > 0),
    })
}

/// HMS code as Bambu writes it: attr and code as two 16-bit halves each.
pub fn hms_code(attr: i64, code: i64) -> String {
    format!("{:04X}_{:04X}_{:04X}_{:04X}", (attr >> 16) & 0xFFFF, attr & 0xFFFF, (code >> 16) & 0xFFFF, code & 0xFFFF)
}

/// Plain words for HMS codes that stop AirSlice from starting a print.
pub fn hms_explained(code: &str) -> Option<&'static str> {
    match code {
        "0500_0500_0001_0007" => Some(
            "the printer refused an unsigned command. Turn on LAN Only Mode and then Developer Mode on the printer (Settings, Network); without Developer Mode, current firmware only accepts commands signed by Bambu's own apps",
        ),
        _ => None,
    }
}

/// Build the view from the merged `print` object.
pub fn view(print: &Value) -> StatusView {
    let mut s = StatusView {
        state: string(print.get("gcode_state")),
        task_name: string(print.get("subtask_name")),
        percent: int(print.get("mc_percent")),
        remaining_minutes: int(print.get("mc_remaining_time")),
        layer: int(print.get("layer_num")),
        total_layers: int(print.get("total_layer_num")),
        nozzle_temp: print.get("nozzle_temper").and_then(num),
        nozzle_target: print.get("nozzle_target_temper").and_then(num),
        bed_temp: print.get("bed_temper").and_then(num),
        bed_target: print.get("bed_target_temper").and_then(num),
        print_error: int(print.get("print_error")).filter(|e| *e != 0),
        ..Default::default()
    };

    s.speed_level = int(print.get("spd_lvl")).filter(|l| (1..=4).contains(l));
    // "-52dBm"
    s.wifi_dbm = string(print.get("wifi_signal")).and_then(|w| w.trim_end_matches("dBm").trim().parse().ok());
    // Fans report 0–15.
    let fan = |k: &str| int(print.get(k)).filter(|v| (0..=15).contains(v)).map(|v| ((v as f64) / 15.0 * 100.0 / 10.0).round() as i64 * 10);
    s.fan_part = fan("cooling_fan_speed");
    s.fan_aux = fan("big_fan1_speed");
    s.fan_chamber = fan("big_fan2_speed");
    s.fan_heatbreak = fan("heatbreak_fan_speed");
    s.nozzle_type = string(print.get("nozzle_type")).filter(|t| !t.is_empty());
    s.speed_percent = int(print.get("spd_mag")).filter(|m| *m > 0);
    // -1 / 255 = no stage.
    s.stage = int(print.get("stg_cur")).filter(|v| (0..255).contains(v));
    s.print_type = string(print.get("print_type")).filter(|t| !t.is_empty());
    s.gcode_file = string(print.get("gcode_file")).filter(|t| !t.is_empty());
    s.sdcard = print.get("sdcard").and_then(|v| v.as_bool());
    s.nozzle_diameter = string(print.get("nozzle_diameter")).or_else(|| print.get("nozzle_diameter").and_then(num).map(|d| format!("{d}")));
    s.hms = print
        .get("hms")
        .and_then(|h| h.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| Some(hms_code(int(e.get("attr"))?, int(e.get("code"))?)))
        .collect();
    s.problems = s.hms.iter().map(|c| Problem::new(c)).collect();
    if let Some(lights) = print.get("lights_report").and_then(|l| l.as_array()) {
        s.chamber_light = lights
            .iter()
            .find(|l| l.get("node").and_then(|n| n.as_str()) == Some("chamber_light"))
            .and_then(|l| l.get("mode").and_then(|m| m.as_str()))
            .map(|m| m == "on");
    }

    let ams_root = print.get("ams");
    let active = ams_root.and_then(|a| int(a.get("tray_now")));
    if let Some(units) = ams_root.and_then(|a| a.get("ams")).and_then(|a| a.as_array()) {
        for unit in units {
            let Some(ams_id) = int(unit.get("id")).map(|i| i as u32) else { continue };
            s.has_ams = true;
            s.ams_units.push(AmsUnit {
                id: ams_id,
                name: if ams_id >= 128 { format!("AMS HT {}", ams_id - 127) } else { format!("AMS {}", (b'A' + (ams_id % 26) as u8) as char) },
                humidity_pct: int(unit.get("humidity_raw")).filter(|h| (0..=100).contains(h)),
                humidity_level: int(unit.get("humidity")).filter(|h| (1..=5).contains(h)),
                temp: unit.get("temp").and_then(num).filter(|t| *t > -40.0 && *t < 120.0 && *t != 0.0),
                dry_minutes: int(unit.get("dry_time")).filter(|m| *m > 0),
                model: None,
                dry_max_temp: None,
                in_print: false,
            });
            for t in unit.get("tray").and_then(|t| t.as_array()).into_iter().flatten() {
                if let Some(tray) = parse_tray(t, Some(ams_id), active) {
                    s.trays.push(tray);
                }
            }
        }
    }
    s.trays.sort_by_key(|t| t.id);

    // External spool: `vt_tray` (most firmware) or `vir_slot` (newer).
    let ext = print
        .get("vt_tray")
        .cloned()
        .or_else(|| print.get("vir_slot").and_then(|v| v.as_array()).and_then(|a| a.first().cloned()));
    if let Some(ext) = ext
        && let Some(mut tray) = parse_tray(&ext, None, active) {
            tray.id = EXTERNAL_TRAY;
            // tray_now 255 means "nothing loaded", 254 is the external spool.
            tray.active = active == Some(254);
            if tray.kind.is_empty() {
                tray.color.clear();
            }
            s.external = Some(tray);
        }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_partial_updates() {
        let mut base = json!({"gcode_state": "IDLE", "ams": {"tray_now": "255", "ams": [1]}, "mc_percent": 0});
        merge(&mut base, &json!({"gcode_state": "RUNNING", "ams": {"tray_now": "2"}}));
        assert_eq!(base["gcode_state"], "RUNNING");
        assert_eq!(base["ams"]["tray_now"], "2");
        assert_eq!(base["ams"]["ams"], json!([1]));
        assert_eq!(base["mc_percent"], 0);
    }

    #[test]
    fn knows_which_ams_units_can_dry() {
        let models = [("n3f/0", "AMS 2 Pro", Some(65)), ("n3s/128", "AMS HT", Some(85)), ("ams/1", "AMS", None), ("ams_f1/0", "AMS Lite", None)];
        for (name, model, max) in models {
            let (id, m) = ams_module(name).unwrap();
            assert_eq!(m, model);
            let mut unit = view(&json!({"ams": {"ams": [{"id": id.to_string(), "tray": []}]}})).ams_units.remove(0);
            unit.set_model(&std::collections::HashMap::from([(id, m.to_string())]));
            assert_eq!((unit.model.as_deref(), unit.dry_max_temp), (Some(model), max));
        }
        assert_eq!(ams_module("ota"), None);
        assert_eq!(ams_module("mc/0"), None);
        // Before the module list has come in, only an HT can be told apart.
        let v = view(&json!({"ams": {"ams": [{"id": "0", "tray": []}, {"id": "128", "tray": []}]}}));
        let mut units = v.ams_units;
        units.iter_mut().for_each(|u| u.set_model(&Default::default()));
        assert_eq!((units[0].dry_max_temp, units[1].dry_max_temp), (None, Some(85)));
        assert_eq!((tray_unit(5), tray_unit(128), tray_unit(254), tray_unit(-1)), (Some(1), Some(128), None, None));
    }

    #[test]
    fn reads_drying_values_of_a_spool() {
        let v = view(&json!({"ams": {"ams": [{"id": "0", "dry_time": 95, "tray": [
            {"id": "0", "tray_type": "PLA", "tray_temp": "55", "tray_time": "8"},
            {"id": "1", "tray_type": "PLA", "tray_temp": "0", "tray_time": "0"}
        ]}]}}));
        assert_eq!(v.ams_units[0].dry_minutes, Some(95));
        assert_eq!((v.trays[0].dry_temp, v.trays[0].dry_hours), (Some(55), Some(8)));
        assert_eq!((v.trays[1].dry_temp, v.trays[1].dry_hours), (None, None));
    }

    #[test]
    fn parses_ams_and_external() {
        let print = json!({
            "gcode_state": "RUNNING", "mc_percent": 42, "mc_remaining_time": 17,
            "nozzle_temper": 219.5, "bed_temper": "60",
            "wifi_signal": "-52dBm", "cooling_fan_speed": "15", "big_fan1_speed": "7",
            "heatbreak_fan_speed": "15", "spd_mag": 100, "stg_cur": 14, "nozzle_type": "hardened_steel", "sdcard": true,
            "lights_report": [{"node": "chamber_light", "mode": "on"}],
            "ams": {"tray_now": "5", "ams": [
                {"id": "0", "humidity": "4", "humidity_raw": "18", "temp": "24.5", "tray": [
                    {"id": "0", "tray_type": "PLA", "tray_color": "FFFFFFFF", "remain": 80, "tray_weight": "1000"},
                    {"id": "1"}
                ]},
                {"id": "1", "tray": [{"id": "1", "tray_type": "PETG", "tray_color": "00FF00FF", "remain": -1}]}
            ]},
            "vt_tray": {"id": "254", "tray_type": "TPU", "tray_color": "000000FF"}
        });
        let v = view(&print);
        assert_eq!(v.state.as_deref(), Some("RUNNING"));
        assert!(v.is_busy());
        assert_eq!(v.percent, Some(42));
        assert_eq!(v.bed_temp, Some(60.0));
        assert_eq!(v.chamber_light, Some(true));
        assert_eq!((v.fan_heatbreak, v.speed_percent, v.stage, v.sdcard), (Some(100), Some(100), Some(14), Some(true)));
        assert_eq!(v.nozzle_type.as_deref(), Some("hardened_steel"));
        assert_eq!(hms_code(0x0500_0500, 0x0001_0007), "0500_0500_0001_0007");
        assert!(hms_explained("0500_0500_0001_0007").is_some());
        let p = Problem::new("0500_0500_0001_0007");
        assert_eq!(p.severity, "fatal");
        assert!(p.url.ends_with("/hmscode/0500_0500_0001_0007"), "{}", p.url);
        assert_eq!(Problem::new("0300_0100_0003_0001").severity, "common");
        assert_eq!(v.trays.len(), 3);
        assert_eq!(v.trays[0].color, "#FFFFFF");
        assert_eq!(v.trays[0].remain, Some(80));
        assert_eq!(v.trays[0].spool_g, Some(1000));
        assert_eq!(v.ams_units[0].name, "AMS A");
        assert_eq!(v.ams_units[0].humidity_pct, Some(18));
        assert_eq!(v.ams_units[0].temp, Some(24.5));
        assert_eq!(v.wifi_dbm, Some(-52));
        assert_eq!(v.fan_part, Some(100));
        assert_eq!(v.fan_aux, Some(50));
        assert_eq!(v.trays[2].id, 5);
        assert!(v.trays[2].active);
        assert_eq!(v.trays[2].remain, None);
        let ext = v.external.as_ref().unwrap();
        assert_eq!(ext.id, EXTERNAL_TRAY);
        assert_eq!(ext.kind, "TPU");
        // empty slot excluded from usable trays
        assert_eq!(v.usable_trays().len(), 3);
    }
}
