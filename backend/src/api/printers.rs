use crate::{
    error::{ApiResult, AppError},
    models::{FtpDataMode, Printer, now},
    printer::{TraySetting, ftp, status::{EXTERNAL_TRAY, StatusView}},
    slicer::{BUILD_PLATES, BuildPlate, build_plate},
    state::Shared,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize)]
pub struct PrinterView {
    pub id: String,
    pub name: String,
    pub host: String,
    pub serial: String,
    pub has_access_code: bool,
    pub printer_model: String,
    pub machine_profile: String,
    pub process_profile: Option<String>,
    pub filament_profile: Option<String>,
    pub ftp_data_mode: FtpDataMode,
    pub plates: Vec<String>,
    pub default_plate: Option<String>,
    pub camera: String,
    pub camera_entity: Option<String>,
    pub power_switch: Option<String>,
    pub power_sensor: Option<String>,
    pub energy_sensor: Option<String>,
    pub power_off_after_min: u32,
    /// The plug's state and power draw, when it has one.
    pub power: Option<crate::power::PowerView>,
    pub enabled: bool,
    pub is_default: bool,
    /// Jobs waiting in this printer's queue.
    pub queue_length: usize,
    pub status: StatusView,
    /// The AirSlice job printing now, if the print came from AirSlice.
    pub current_job: Option<JobRef>,
    /// The last print that ended on this printer.
    pub last_job: Option<JobRef>,
}

/// A job, as much as the printer card shows of it.
#[derive(Serialize)]
pub struct JobRef {
    pub id: String,
    pub title: String,
    pub plate_label: Option<String>,
    /// Plate with a thumbnail (`api/jobs/<id>/plates/<n>/thumbnail`).
    pub thumbnail_plate: Option<u32>,
    pub cover: Option<String>,
    pub photo: bool,
    pub outcome: Option<crate::models::PrintOutcome>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

fn job_ref(j: &crate::models::Job) -> JobRef {
    let plate = j.slice.as_ref().map(|s| s.plate);
    let thumbnail_plate = j
        .inspection
        .as_ref()
        .and_then(|i| i.plates.iter().find(|p| Some(p.index) == plate && p.has_thumbnail).or_else(|| i.plates.iter().find(|p| p.has_thumbnail)))
        .map(|p| p.index);
    JobRef {
        id: j.id.clone(),
        title: j.model_title.clone(),
        plate_label: j.plate_label.clone(),
        thumbnail_plate,
        cover: j.model_cover.clone(),
        photo: j.photo,
        outcome: j.print.as_ref().and_then(|p| p.outcome),
        started_at: j.print.as_ref().map(|p| p.started_at),
        ended_at: j.print.as_ref().and_then(|p| p.ended_at),
    }
}

pub async fn view(state: &Shared, p: &Printer, default_id: Option<&str>) -> PrinterView {
    PrinterView {
        id: p.id.clone(),
        name: p.name.clone(),
        host: p.host.clone(),
        serial: p.serial.clone(),
        has_access_code: !p.access_code.is_empty(),
        printer_model: p.printer_model.clone(),
        machine_profile: p.machine_profile.clone(),
        process_profile: p.process_profile.clone(),
        filament_profile: p.filament_profile.clone(),
        ftp_data_mode: p.ftp_data_mode,
        plates: p.plates.clone(),
        default_plate: p.default_plate.clone(),
        camera: p.camera.clone(),
        camera_entity: p.camera_entity.clone(),
        power_switch: p.power_switch.clone(),
        power_sensor: p.power_sensor.clone(),
        energy_sensor: p.energy_sensor.clone(),
        power_off_after_min: p.power_off_after_min,
        power: crate::power::view(state, p).await,
        enabled: p.enabled,
        is_default: default_id == Some(p.id.as_str()),
        // Everything queued for its type: any of those can go to this printer.
        queue_length: state.store.read(|d| d.queue.iter().filter(|q| crate::jobs::queue_group(&d.printers, &q.printer_id) == p.machine_profile).count()).await,
        status: state.printers.status(&p.id).await,
        current_job: state
            .store
            .read(|d| {
                d.jobs
                    .iter()
                    .filter(|j| j.state == crate::models::JobState::Started && j.print.as_ref().is_some_and(|pr| pr.printer_id == p.id))
                    .max_by_key(|j| j.print.as_ref().map(|pr| pr.started_at))
                    .map(job_ref)
            })
            .await,
        last_job: state
            .store
            .read(|d| {
                d.jobs
                    .iter()
                    .filter(|j| j.print.as_ref().is_some_and(|pr| pr.printer_id == p.id && pr.ended_at.is_some()))
                    .max_by_key(|j| j.print.as_ref().and_then(|pr| pr.ended_at))
                    .map(job_ref)
            })
            .await,
    }
}

pub async fn list(State(state): State<Shared>) -> Json<Vec<PrinterView>> {
    let (printers, default_id) = state.store.read(|d| (d.printers.clone(), d.default_printer_id.clone())).await;
    let mut out = Vec::new();
    for p in &printers {
        out.push(view(&state, p, default_id.as_deref()).await);
    }
    Json(out)
}

async fn load(state: &Shared, id: &str) -> ApiResult<Printer> {
    state
        .store
        .read(|d| d.printers.iter().find(|p| p.id == id).cloned())
        .await
        .ok_or_else(|| AppError::not_found("printer not found"))
}

pub async fn get(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<PrinterView>> {
    let p = load(&state, &id).await?;
    let default_id = state.store.read(|d| d.default_printer_id.clone()).await;
    Ok(Json(view(&state, &p, default_id.as_deref()).await))
}

#[derive(Deserialize)]
pub struct PrinterInput {
    name: String,
    host: String,
    serial: String,
    /// Leave empty when editing to keep the stored code.
    #[serde(default)]
    access_code: Option<String>,
    machine_profile: String,
    #[serde(default)]
    printer_model: Option<String>,
    #[serde(default)]
    process_profile: Option<String>,
    #[serde(default)]
    filament_profile: Option<String>,
    #[serde(default)]
    ftp_data_mode: FtpDataMode,
    #[serde(default)]
    plates: Vec<String>,
    #[serde(default)]
    default_plate: Option<String>,
    #[serde(default)]
    camera: String,
    #[serde(default)]
    camera_entity: Option<String>,
    #[serde(default)]
    power_switch: Option<String>,
    #[serde(default)]
    power_off_after_min: u32,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// "Bambu Lab P1S 0.4 nozzle" -> "Bambu Lab P1S"
fn model_from_machine(machine: &str) -> String {
    match machine.rfind(" nozzle") {
        Some(_) => machine.rsplitn(3, ' ').nth(2).unwrap_or(machine).to_string(),
        None => machine.to_string(),
    }
}

async fn validate(state: &Shared, input: PrinterInput, existing: Option<&Printer>) -> ApiResult<Printer> {
    let name = input.name.trim().to_string();
    let host = input.host.trim().to_string();
    let serial = input.serial.trim().to_uppercase();
    let machine = input.machine_profile.trim().to_string();
    if name.is_empty() || host.is_empty() || serial.is_empty() || machine.is_empty() {
        return Err(AppError::bad_request("name, IP address, serial number and printer profile are required"));
    }
    let looks_like_port = host.matches(':').count() == 1;
    if host.contains('/') || host.contains(char::is_whitespace) || looks_like_port {
        return Err(AppError::bad_request("enter just the printer's IP address or host name"));
    }
    let access_code = match clean(input.access_code) {
        Some(c) => c,
        None => existing.map(|e| e.access_code.clone()).filter(|c| !c.is_empty()).ok_or_else(|| {
            AppError::bad_request("the access code is required (Settings → LAN Only Mode on the printer)")
        })?,
    };
    // Prefer the model name from the slicer profile itself.
    let printer_model = match state.slicer.profiles().await {
        Ok(index) => {
            if !index.has_machine(&machine) {
                return Err(AppError::bad_request(format!("unknown printer profile {machine:?}")));
            }
            index.machines().into_iter().find(|m| m.name == machine).map(|m| m.printer_model).unwrap_or_default()
        }
        Err(_) => String::new(),
    };
    let printer_model = if printer_model.is_empty() {
        clean(input.printer_model).unwrap_or_else(|| model_from_machine(&machine))
    } else {
        printer_model
    };
    let (plates, default_plate) = validate_plates(input.plates, clean(input.default_plate))?;
    let camera = match input.camera.trim() {
        "" | "off" => String::new(),
        c @ ("printer" | "ha") => c.to_string(),
        other => return Err(AppError::bad_request(format!("unknown camera source {other:?}"))),
    };
    let camera_entity = clean(input.camera_entity).filter(|_| camera == "ha");
    if camera == "ha" && !camera_entity.as_deref().is_some_and(|e| crate::notify::is_entity_id(e, "camera")) {
        return Err(AppError::bad_request("choose the Home Assistant camera (camera.…)"));
    }
    let power_switch = clean(input.power_switch);
    if power_switch.as_deref().is_some_and(|e| !crate::notify::is_entity_id(e, "switch")) {
        return Err(AppError::bad_request("choose the plug's switch in Home Assistant (switch.…)"));
    }
    // Look up the plug's sensors when the switch is new or changed.
    let (power_sensor, energy_sensor) = match (&power_switch, existing) {
        (None, _) => (None, None),
        (Some(sw), Some(e)) if e.power_switch.as_ref() == Some(sw) && (e.power_sensor.is_some() || e.energy_sensor.is_some()) => {
            (e.power_sensor.clone(), e.energy_sensor.clone())
        }
        (Some(sw), _) => crate::power::find_sensors(state, sw).await,
    };
    Ok(Printer {
        id: existing.map(|e| e.id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()[..12].to_string()),
        name,
        host,
        serial,
        access_code,
        printer_model,
        machine_profile: machine,
        process_profile: clean(input.process_profile),
        filament_profile: clean(input.filament_profile),
        ftp_data_mode: input.ftp_data_mode,
        plates,
        default_plate,
        camera,
        camera_entity,
        power_off_after_min: if power_switch.is_some() { input.power_off_after_min.min(24 * 60) } else { 0 },
        power_switch,
        power_sensor,
        energy_sensor,
        enabled: input.enabled,
        created_at: existing.map(|e| e.created_at).unwrap_or_else(now),
    })
}

/// Known plate values, without repeats, in the slicer's spelling; the default
/// must be one of them (and is added when the list is empty).
fn validate_plates(plates: Vec<String>, default: Option<String>) -> ApiResult<(Vec<String>, Option<String>)> {
    let known = |p: &str| build_plate(p).map(|b| b.value.to_string()).ok_or_else(|| AppError::bad_request(format!("unknown build plate {p:?}")));
    let mut out: Vec<String> = Vec::new();
    for p in &plates {
        let v = known(p)?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    let default = default.map(|d| known(&d)).transpose()?;
    if let Some(d) = &default {
        if out.is_empty() {
            out.push(d.clone());
        } else if !out.contains(d) {
            return Err(AppError::bad_request("the default plate must be one of the printer's plates"));
        }
    }
    Ok((out, default))
}

/// The latest camera picture.
pub async fn camera(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let p = load(&state, &id).await?;
    let img = crate::camera::snapshot(&state, &p).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "image/jpeg"), (axum::http::header::CACHE_CONTROL, "no-store")],
        img.as_ref().clone(),
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct PowerBody {
    on: bool,
}

/// Turn the printer's plug on or off.
pub async fn power(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<PowerBody>) -> ApiResult<Json<Value>> {
    let p = load(&state, &id).await?;
    if !body.on && state.printers.status(&p.id).await.is_busy() {
        return Err(AppError::bad_request(format!("{} is printing; stop the print first", p.name)));
    }
    crate::power::set(&state, &p, body.on).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

/// Home Assistant switches, for the printer form's power plug.
pub async fn ha_switches(State(state): State<Shared>) -> ApiResult<Json<Vec<Value>>> {
    ha_entities(&state, "switch.").await.map(Json)
}

/// Home Assistant camera entities, for the printer form.
pub async fn ha_cameras(State(state): State<Shared>) -> ApiResult<Json<Vec<Value>>> {
    ha_entities(&state, "camera.").await.map(Json)
}

async fn ha_entities(state: &Shared, prefix: &str) -> ApiResult<Vec<Value>> {
    let settings = state.store.read(|d| d.notify.clone()).await;
    let (base, token) = crate::notify::endpoint(&settings).map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    let states: Vec<Value> = state
        .http
        .get(format!("{base}/states"))
        .bearer_auth(token)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AppError::bad_gateway(format!("Home Assistant: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::bad_gateway(format!("Home Assistant: {e}")))?;
    Ok(states
            .into_iter()
            .filter(|s| s.get("entity_id").and_then(|e| e.as_str()).is_some_and(|e| e.starts_with(prefix)))
            .map(|s| json!({ "entity_id": s["entity_id"], "name": s.pointer("/attributes/friendly_name").cloned().unwrap_or(s["entity_id"].clone()) }))
            .collect())
}

/// Build plates OrcaSlicer knows.
pub async fn plates() -> Json<Vec<BuildPlate>> {
    Json(BUILD_PLATES.to_vec())
}

async fn resync(state: &Shared) {
    let printers = state.store.read(|d| d.printers.clone()).await;
    state.printers.sync(&printers).await;
}

pub async fn create(State(state): State<Shared>, Json(input): Json<PrinterInput>) -> ApiResult<Json<PrinterView>> {
    let printer = validate(&state, input, None).await?;
    let p = printer.clone();
    let default_id = state
        .store
        .update(move |d| {
            if d.printers.iter().any(|x| x.serial == p.serial) {
                return Err(AppError::conflict("a printer with this serial number already exists"));
            }
            d.printers.push(p.clone());
            if d.default_printer_id.is_none() {
                d.default_printer_id = Some(p.id.clone());
            }
            Ok(d.default_printer_id.clone())
        })
        .await??;
    resync(&state).await;
    Ok(Json(view(&state, &printer, default_id.as_deref()).await))
}

pub async fn update(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(input): Json<PrinterInput>,
) -> ApiResult<Json<PrinterView>> {
    let existing = load(&state, &id).await?;
    let printer = validate(&state, input, Some(&existing)).await?;
    let p = printer.clone();
    let default_id = state
        .store
        .update(move |d| {
            if d.printers.iter().any(|x| x.serial == p.serial && x.id != p.id) {
                return Err(AppError::conflict("another printer has this serial number"));
            }
            if let Some(slot) = d.printers.iter_mut().find(|x| x.id == p.id) {
                *slot = p;
            }
            Ok(d.default_printer_id.clone())
        })
        .await??;
    resync(&state).await;
    Ok(Json(view(&state, &printer, default_id.as_deref()).await))
}

pub async fn delete(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    state
        .store
        .update(|d| {
            d.printers.retain(|p| p.id != id);
            let queued: Vec<String> = d.queue.iter().filter(|q| q.printer_id == id).map(|q| q.job_id.clone()).collect();
            d.queue.retain(|q| q.printer_id != id);
            for j in d.jobs.iter_mut().filter(|j| queued.contains(&j.id)) {
                j.state = crate::models::JobState::Sliced;
            }
            if d.default_printer_id.as_deref() == Some(id.as_str()) {
                d.default_printer_id = d.printers.first().map(|p| p.id.clone());
            }
        })
        .await?;
    resync(&state).await;
    Ok(Json(json!({"ok": true})))
}

pub async fn make_default(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    state.store.update(|d| d.default_printer_id = Some(id)).await?;
    Ok(Json(json!({"ok": true})))
}

/// Check both channels the app uses: MQTT (status/commands) and FTPS (upload).
pub async fn test(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let p = load(&state, &id).await?;
    let mut status = state.printers.status(&id).await;
    if status.connected {
        let _ = state.printers.request_full_status(&id).await;
    } else {
        // Give a fresh connection a moment.
        for _ in 0..8 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            status = state.printers.status(&id).await;
            if status.connected {
                break;
            }
        }
    }
    let target = ftp::FtpTarget { host: &p.host, access_code: &p.access_code };
    let ftp_result = ftp::check_login(&target).await;
    let refused = if status.connected { state.printers.commands_refused(&id).await } else { None };
    Ok(Json(json!({
        "mqtt": { "ok": status.connected, "error": status.last_error },
        "commands": { "ok": refused.is_none(), "error": refused },
        "ftp": { "ok": ftp_result.is_ok(), "error": ftp_result.err().map(|e| format!("{e:#}")) },
    })))
}

#[derive(Deserialize)]
pub struct ControlBody {
    action: String,
}

pub async fn control(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<ControlBody>,
) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    state.printers.control(&id, &body.action).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct TrayBody {
    /// OrcaSlicer filament profile, e.g. "Generic PLA @BBL X1C".
    profile: String,
    /// `#RRGGBB`
    color: String,
}

/// Set what is loaded in a tray (spools without RFID, or a wrong read).
pub async fn set_tray(
    State(state): State<Shared>,
    Path((id, tray)): Path<(String, i32)>,
    Json(body): Json<TrayBody>,
) -> ApiResult<Json<Value>> {
    let p = load(&state, &id).await?;
    known_tray(&state, &id, tray).await?;
    let color = crate::threemf::hex_color(&body.color);
    if color.is_empty() {
        return Err(AppError::bad_request("choose a colour"));
    }
    let index = state.slicer.profiles().await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    let facts = index
        .filament_facts(&p.machine_profile, &body.profile)
        .ok_or_else(|| AppError::bad_request(format!("{} is not a filament profile for this printer", body.profile)))?;
    let setting = TraySetting { kind: facts.kind, color, info_idx: facts.filament_id, temp_min: facts.temp_min, temp_max: facts.temp_max };
    state.printers.set_tray(&id, tray, &setting).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

/// Spoolman spools: which one is in each tray, and all of them for linking.
pub async fn spools(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    let s = state.store.read(|d| d.spoolman.clone()).await;
    if s.url.trim().is_empty() {
        return Ok(Json(json!({ "configured": false })));
    }
    let all = crate::spoolman::spools(&state, &s).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    let status = state.printers.status(&id).await;
    let trays = crate::spoolman::tray_spools(&id, &status.usable_trays(), &all, &s.links);
    let linked: Value = trays.iter().map(|(t, sp)| (t.to_string(), json!(sp))).collect::<serde_json::Map<_, _>>().into();
    Ok(Json(json!({ "configured": true, "trays": linked, "spools": all })))
}

#[derive(Deserialize)]
pub struct SpoolLink {
    spool_id: Option<i64>,
}

/// Link a tray to a Spoolman spool by hand (spools without an RFID tag).
pub async fn link_spool(
    State(state): State<Shared>,
    Path((id, tray)): Path<(String, i32)>,
    Json(body): Json<SpoolLink>,
) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    let key = crate::spoolman::link_key(&id, tray);
    state
        .store
        .update(|d| match body.spool_id {
            Some(sid) => {
                // A spool sits in one tray at a time.
                d.spoolman.links.retain(|_, v| *v != sid);
                d.spoolman.links.insert(key.clone(), sid);
            }
            None => {
                d.spoolman.links.remove(&key);
            }
        })
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// Have the AMS read a spool's RFID tag again.
pub async fn reread_tray(State(state): State<Shared>, Path((id, tray)): Path<(String, i32)>) -> ApiResult<Json<Value>> {
    load(&state, &id).await?;
    known_tray(&state, &id, tray).await?;
    state.printers.reread_tray(&id, tray).await.map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

async fn known_tray(state: &Shared, id: &str, tray: i32) -> ApiResult<()> {
    let status = state.printers.status(id).await;
    if tray == EXTERNAL_TRAY || status.trays.iter().any(|t| t.id == tray) {
        Ok(())
    } else {
        Err(AppError::bad_request("the printer isn't reporting that tray"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plate_settings() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (plates, default) = validate_plates(v(&["textured pei plate", "SuperTack Plate", "Textured PEI Plate"]), Some("supertack plate".into())).unwrap();
        assert_eq!(plates, v(&["Textured PEI Plate", "Supertack Plate"]));
        assert_eq!(default.as_deref(), Some("Supertack Plate"));
        assert_eq!(validate_plates(vec![], Some("Cool Plate".into())).unwrap(), (v(&["Cool Plate"]), Some("Cool Plate".into())));
        assert!(validate_plates(v(&["Cool Plate"]), Some("Engineering Plate".into())).is_err());
        assert!(validate_plates(v(&["Glass"]), None).is_err());
    }

    #[test]
    fn machine_to_model() {
        assert_eq!(model_from_machine("Bambu Lab P1S 0.4 nozzle"), "Bambu Lab P1S");
        assert_eq!(model_from_machine("Bambu Lab A1 mini 0.2 nozzle"), "Bambu Lab A1 mini");
        assert_eq!(model_from_machine("Custom"), "Custom");
    }
}

#[derive(Serialize)]
pub struct TrayFilament {
    tray_id: i32,
    #[serde(rename = "type")]
    kind: String,
    color: String,
    /// OrcaSlicer filament profile AirSlice would slice with.
    profile: Option<String>,
    /// True when matched by the spool's RFID, not just the material.
    exact: bool,
}

/// Which filament profile each loaded tray maps to.
pub async fn tray_filaments(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Vec<TrayFilament>>> {
    let p = load(&state, &id).await?;
    let status = state.printers.status(&id).await;
    let index = state.slicer.profiles().await.ok();
    let out = status
        .usable_trays()
        .into_iter()
        .map(|t| {
            let profile = index.as_ref().and_then(|i| i.filament_for_tray(&p.machine_profile, t.info_idx.as_deref(), &t.kind));
            let exact = match (&index, &t.info_idx) {
                (Some(i), Some(idx)) => i
                    .choices_for(&p.machine_profile)
                    .filaments
                    .iter()
                    .any(|f| f.filament_id.as_deref() == Some(idx.as_str()) && Some(&f.name) == profile.as_ref()),
                _ => false,
            };
            TrayFilament { tray_id: t.id, kind: t.kind.clone(), color: t.color.clone(), profile, exact }
        })
        .collect();
    Ok(Json(out))
}
