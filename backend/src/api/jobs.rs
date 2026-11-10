use crate::{
    error::{ApiResult, AppError},
    jobs,
    models::{Job, JobState, PrintOptions, ResolvedModel, SliceOptions},
    printer::{
        mapping::{self, MappingSuggestion},
        status::Tray,
    },
    state::Shared,
    threemf,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

fn bad(e: anyhow::Error) -> AppError {
    AppError::bad_request(format!("{e:#}"))
}

#[derive(Deserialize)]
pub struct ResolveBody {
    url: String,
}

/// The linked Bambu account's token and region, or a message to link one.
async fn bambu(state: &Shared) -> ApiResult<(bool, String)> {
    let acct = state.store.read(|d| d.bambu.clone()).await;
    let token = acct.token.ok_or_else(|| AppError::bad_request("Link your Bambu account under Settings → Model sites to see your MakerWorld collections."))?;
    Ok((acct.region == "china", token))
}

/// The signed-in account's MakerWorld collections.
pub async fn collections(State(state): State<Shared>) -> ApiResult<Json<Vec<crate::sources::makerworld::Collection>>> {
    let (china, token) = bambu(&state).await?;
    let list = crate::sources::makerworld::collections(&state.http, china, &token).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(list))
}

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    limit: Option<u32>,
}

/// A page of models in one collection.
pub async fn collection(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<crate::sources::makerworld::CollectionPage>> {
    let (china, token) = bambu(&state).await?;
    let page = crate::sources::makerworld::collection_models(&state.http, china, &token, &id, q.offset, q.limit.unwrap_or(24))
        .await
        .map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(page))
}

pub async fn resolve(State(state): State<Shared>, Json(body): Json<ResolveBody>) -> ApiResult<Json<ResolvedModel>> {
    let model = jobs::resolve(&state, &body.url).await.map_err(|e| {
        let msg = format!("{e:#}");
        if msg.contains("Paste a") || msg.contains("Only MakerWorld") || msg.contains("Open a specific") {
            AppError::bad_request(msg)
        } else {
            AppError::bad_gateway(msg)
        }
    })?;
    Ok(Json(model))
}

#[derive(Serialize)]
pub struct JobView {
    #[serde(flatten)]
    job: Job,
    /// For a sliced job: suggested trays for each filament and what is loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<MappingSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trays: Option<Vec<Tray>>,
    /// While slicing: how far OrcaSlicer has come.
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<crate::models::SliceProgress>,
    /// While printing: the printer's live progress for this print.
    #[serde(skip_serializing_if = "Option::is_none")]
    live: Option<Value>,
    /// Spoolman spools in the printer's trays (tray id -> spool).
    #[serde(skip_serializing_if = "Option::is_none")]
    spools: Option<HashMap<i32, crate::spoolman::Spool>>,
    /// Cost estimate for a sliced job (finished jobs carry `cost`).
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_estimate: Option<crate::models::Cost>,
    /// Sent to the printer: the next plate of the file nobody printed or queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_plate: Option<crate::models::PlateInfo>,
}

async fn view(state: &Shared, job: Job) -> JobView {
    let (mut suggestion, mut trays) = (None, None);
    if let (Some(slice), JobState::Sliced) = (&job.slice, job.state) {
        let status = state.printers.status(&slice.printer_id).await;
        suggestion = Some(mapping::suggest_with(
            &slice.filaments,
            slice.filament_slots,
            &status,
            slice.preferred_trays.as_deref().unwrap_or(&[]),
        ));
        trays = Some(status.usable_trays().into_iter().cloned().collect());
    }
    let progress = matches!(job.state, JobState::Slicing | JobState::Uploading).then(|| state.progress(&job.id)).flatten();
    let mut live = None;
    if let (Some(p), JobState::Started) = (&job.print, job.state) {
        let s = state.printers.status(&p.printer_id).await;
        if s.connected && p.is_task(s.task_name.as_deref()) {
            live = Some(json!({
                "state": s.state, "percent": s.percent, "remaining_minutes": s.remaining_minutes,
                "layer": s.layer, "total_layers": s.total_layers,
            }));
        }
    }
    let (mut spools, mut cost_estimate) = (None, None);
    if let (Some(slice), JobState::Sliced | JobState::Queued | JobState::Started) = (&job.slice, job.state) {
        let printer_spools = crate::spoolman::for_printer(state, &slice.printer_id).await;
        let mapping = job
            .print
            .as_ref()
            .map(|p| p.ams_mapping.clone())
            .or_else(|| suggestion.as_ref().map(|s| s.ams_mapping.clone()))
            .unwrap_or_default();
        let costs = state.store.read(|d| d.cost.clone()).await;
        let use_g = crate::spoolman::use_per_tray(&slice.filaments, &mapping, 1.0);
        let hours = slice.prediction_secs.map(|s| s as f64 / 3600.0);
        let prices = crate::spoolman::prices_for_printer(state, &slice.printer_id).await;
        cost_estimate = crate::spoolman::cost(&use_g, &prices, hours, &costs, false);
        if !printer_spools.is_empty() {
            spools = Some(printer_spools);
        }
    }
    let next_plate = jobs::next_plate(state, &job).await;
    JobView { job, suggestion, trays, progress, live, spools, cost_estimate, next_plate }
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn list(State(state): State<Shared>, Query(q): Query<ListQuery>) -> Json<Vec<Job>> {
    let limit = q.limit.unwrap_or(50);
    Json(state.store.read(|d| d.jobs.iter().take(limit).cloned().collect()).await)
}

pub async fn get(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    Ok(Json(view(&state, job).await))
}

#[derive(Deserialize)]
pub struct CreateBody {
    url: String,
    #[serde(default)]
    file_id: Option<String>,
}

pub async fn create(State(state): State<Shared>, Json(body): Json<CreateBody>) -> ApiResult<Json<JobView>> {
    let job = jobs::create(&state, &body.url, body.file_id.as_deref(), None).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn slice(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(opts): Json<SliceOptions>,
) -> ApiResult<Json<JobView>> {
    let job = jobs::slice(&state, &id, opts).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn print(
    State(state): State<Shared>,
    Path(id): Path<String>,
    body: Option<Json<PrintOptions>>,
) -> ApiResult<Json<JobView>> {
    let opts = body.map(|b| b.0).unwrap_or_default();
    let job = jobs::print(&state, &id, opts).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn delete(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    jobs::delete(&state, &id).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn enqueue(
    State(state): State<Shared>,
    Path(id): Path<String>,
    body: Option<Json<PrintOptions>>,
) -> ApiResult<Json<JobView>> {
    let job = jobs::enqueue(&state, &id, body.map(|b| b.0).unwrap_or_default()).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

#[derive(Deserialize)]
pub struct FavoriteBody {
    favorite: bool,
}

pub async fn favorite(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<FavoriteBody>,
) -> ApiResult<Json<JobView>> {
    let job = jobs::set_favorite(&state, &id, body.favorite).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

/// Start page hint: the next plate of the last print, when nothing is queued.
pub async fn next_plate_suggestion(State(state): State<Shared>) -> Json<Value> {
    Json(match jobs::suggest_next_plate(&state).await {
        Some((job, plate)) => json!({
            "job_id": job.id, "model_title": job.model_title, "model_cover": job.model_cover,
            "has_photo": job.photo, "plate": plate,
            "plates": job.inspection.as_ref().map(|i| i.plates.len()).unwrap_or(0),
        }),
        None => Value::Null,
    })
}

pub async fn queue_next_plate(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::queue_next_plate(&state, &id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn reprint(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::reprint(&state, &id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

/// The sliced plate's G-code, copied out and indexed on first use.
async fn layer_index(state: &Shared, id: &str) -> ApiResult<(std::path::PathBuf, crate::gcode::Index)> {
    let job = jobs::get_job(state, id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let slice = job.slice.clone().ok_or_else(|| AppError::bad_request("not sliced yet"))?;
    let dir = jobs::job_dir(state, id);
    let sliced = job.output_file.as_ref().map(|f| dir.join(f)).filter(|p| p.exists()).ok_or_else(|| AppError::not_found("the sliced file was removed"))?;
    tokio::task::spawn_blocking(move || crate::gcode::load_index(&sliced, &slice.gcode_entry, &dir, slice.plate))
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
        .map_err(|e| {
            tracing::warn!(job = id, "layer view failed: {e:#}");
            AppError::bad_request(format!("{e:#}"))
        })
}

/// Layers of the sliced plate (heights, extruded area, feature names).
pub async fn layers(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<crate::gcode::Index>> {
    Ok(Json(layer_index(&state, &id).await?.1.public()))
}

/// Extrusion paths of one layer: `[feature, tool, x, y, x, y, …]` in 1/100 mm.
pub async fn layer(State(state): State<Shared>, Path((id, n)): Path<(String, usize)>) -> ApiResult<axum::response::Response> {
    let (gcode, index) = layer_index(&state, &id).await?;
    let z = index.layers.get(n).map(|l| l.z).ok_or_else(|| AppError::not_found("no such layer"))?;
    let paths = tokio::task::spawn_blocking(move || crate::gcode::read_layer(&gcode, &index, n))
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
        .map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    Ok(([(header::CACHE_CONTROL, "private, max-age=300")], Json(json!({ "z": z, "paths": paths }))).into_response())
}

/// The picture taken when the print ended.
/// Ended prints, newest first: the print history for Home Assistant and others.
#[derive(Deserialize)]
pub struct PrintsQuery {
    printer_id: Option<String>,
    limit: Option<usize>,
}

pub async fn prints(State(state): State<Shared>, Query(q): Query<PrintsQuery>) -> Json<Vec<crate::prints::PrintRecord>> {
    Json(crate::prints::list(&state, q.printer_id.as_deref(), q.limit.unwrap_or(usize::MAX)).await)
}

pub async fn import_print(
    State(state): State<Shared>,
    Json(body): Json<crate::prints::ImportPrint>,
) -> ApiResult<Json<crate::prints::ImportResult>> {
    Ok(Json(crate::prints::import(&state, body).await.map_err(bad)?))
}

/// The picture of an imported or external print, as JPEG bytes.
pub async fn set_photo(State(state): State<Shared>, Path(id): Path<String>, body: axum::body::Bytes) -> ApiResult<Json<JobView>> {
    let job = crate::prints::set_photo(&state, &id, &body).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn photo(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<axum::response::Response> {
    let path = state.cfg.data_dir.join("photos").join(format!("{}.jpg", id.replace(['/', '.'], "")));
    let bytes = tokio::fs::read(path).await.map_err(|_| AppError::not_found("no photo for this print"))?;
    Ok(([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "max-age=86400")], bytes).into_response())
}

/// Stop waiting for an offline printer.
pub async fn cancel_wait(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::cancel_wait(&state, &id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn reslice(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::reslice(&state, &id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

#[derive(Deserialize)]
pub struct TraysBody {
    ams_mapping: Vec<i32>,
    /// Trays for added models (id -> tray), which can differ from their slot's.
    #[serde(default)]
    extra_trays: std::collections::HashMap<String, i32>,
}

/// Slice a sliced job again with the trays now chosen.
pub async fn slice_with_trays(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<TraysBody>) -> ApiResult<Json<JobView>> {
    let job = jobs::slice_with_trays(&state, &id, &body.ams_mapping, &body.extra_trays).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

/// The cost of a sliced job with other trays, which needs no new slice.
pub async fn cost_with_trays(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<TraysBody>) -> ApiResult<Json<Option<crate::models::Cost>>> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let slice = job.slice.ok_or_else(|| AppError::bad_request("this job isn't sliced"))?;
    let costs = state.store.read(|d| d.cost.clone()).await;
    let use_g = crate::spoolman::use_per_tray(&slice.filaments, &body.ams_mapping, 1.0);
    let hours = slice.prediction_secs.map(|s| s as f64 / 3600.0);
    let prices = crate::spoolman::prices_for_printer(&state, &slice.printer_id).await;
    Ok(Json(crate::spoolman::cost(&use_g, &prices, hours, &costs, false)))
}

#[derive(Deserialize)]
pub struct EventsQuery {
    after: Option<u64>,
}

/// What happened since `after`, for clients that poll.
pub async fn events(Query(q): Query<EventsQuery>) -> Json<Value> {
    let (latest, events) = crate::event_log::since(q.after);
    Json(json!({ "latest": latest, "events": events }))
}

#[derive(Deserialize)]
pub struct FilamentCheckBody {
    /// The trays on screen; absent = the queued ones, or the printer's match.
    #[serde(default)]
    ams_mapping: Option<Vec<i32>>,
}

/// Whether the spools will last for this print.
pub async fn filament_check(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<FilamentCheckBody>) -> ApiResult<Json<crate::filament_check::Check>> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let slice = job.slice.as_ref().ok_or_else(|| AppError::bad_request("this job isn't sliced"))?;
    let queued = state.store.read(|d| d.queue.iter().find(|q| q.job_id == id).map(|q| q.printer_id.clone())).await;
    let printer = queued.unwrap_or_else(|| slice.printer_id.clone());
    Ok(Json(crate::filament_check::for_job(&state, &job, &printer, body.ams_mapping).await))
}

// ------------------------------------------------------------------ queue

#[derive(Serialize)]
pub struct QueueView {
    id: String,
    job_id: String,
    /// The printer it was sliced for or last moved to.
    printer_id: String,
    /// The queue it is in: the printer profile it was sliced for (any printer
    /// with that profile can print it).
    group: String,
    added_at: i64,
    /// 1-based position in its group.
    position: usize,
    title: String,
    cover: Option<String>,
    file_name: String,
    plate: Option<u32>,
    prediction_secs: Option<u64>,
    weight_g: Option<f64>,
    filaments: Vec<crate::models::SlicedFilament>,
    has_thumbnail: bool,
    plate_label: Option<String>,
    /// Came sliced: can't be sliced again for another printer type.
    presliced: bool,
    /// Whether the loaded filament matches (or a person chose the trays).
    filament_ok: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    filament_warnings: Vec<String>,
}

#[derive(Deserialize)]
pub struct QueueQuery {
    #[serde(default)]
    printer_id: Option<String>,
}

pub async fn queue(State(state): State<Shared>, Query(q): Query<QueueQuery>) -> Json<Vec<QueueView>> {
    let mut out: Vec<QueueView> = state
        .store
        .read(|d| {
            let mut positions: std::collections::HashMap<String, usize> = Default::default();
            d.queue
                .iter()
                .filter_map(|item| {
                    let group = jobs::queue_group(&d.printers, &item.printer_id);
                    let pos = positions.entry(group.clone()).or_default();
                    *pos += 1;
                    // A printer's queue: everything of its type.
                    if q.printer_id.as_deref().is_some_and(|p| jobs::queue_group(&d.printers, p) != group) {
                        return None;
                    }
                    let job = d.jobs.iter().find(|j| j.id == item.job_id)?;
                    let slice = job.slice.as_ref();
                    Some(QueueView {
                        id: item.id.clone(),
                        job_id: job.id.clone(),
                        printer_id: item.printer_id.clone(),
                        group,
                        added_at: item.added_at,
                        position: *pos,
                        title: job.model_title.clone(),
                        cover: job.model_cover.clone(),
                        file_name: job.file.name.clone(),
                        plate: slice.map(|s| s.plate),
                        prediction_secs: slice.and_then(|s| s.prediction_secs),
                        weight_g: slice.and_then(|s| s.weight_g),
                        filaments: slice.map(|s| s.filaments.clone()).unwrap_or_default(),
                        has_thumbnail: job
                            .inspection
                            .as_ref()
                            .and_then(|i| i.plates.iter().find(|p| Some(p.index) == slice.map(|s| s.plate)))
                            .map(|p| p.has_thumbnail)
                            .unwrap_or(false),
                        plate_label: job.plate_label.clone(),
                        presliced: slice.is_some_and(|s| s.settings == "presliced") || job.inspection.as_ref().is_some_and(|i| i.kind == "sliced"),
                        filament_ok: true,
                        filament_warnings: vec![],
                    })
                })
                .collect()
        })
        .await;
    // Check filament against what's loaded right now.
    let mut statuses = std::collections::HashMap::new();
    for v in out.iter_mut() {
        let (explicit, slice) = state
            .store
            .read(|d| {
                (
                    d.queue.iter().find(|q| q.id == v.id).map(|q| q.options.ams_mapping.is_some()).unwrap_or(false),
                    d.jobs.iter().find(|j| j.id == v.job_id).and_then(|j| j.slice.clone()),
                )
            })
            .await;
        if explicit {
            continue;
        }
        let Some(slice) = slice else { continue };
        if !statuses.contains_key(&v.printer_id) {
            statuses.insert(v.printer_id.clone(), state.printers.status(&v.printer_id).await);
        }
        let s = mapping::suggest_with(&slice.filaments, slice.filament_slots, &statuses[&v.printer_id], slice.preferred_trays.as_deref().unwrap_or(&[]));
        v.filament_ok = s.complete;
        if !s.complete {
            v.filament_warnings = s.warnings;
        }
    }
    Json(out)
}

pub async fn dequeue(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    jobs::dequeue(&state, &id).await.map_err(|e| AppError::not_found(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct MoveBody {
    /// "up" or "down"
    direction: String,
}

pub async fn move_item(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<MoveBody>,
) -> ApiResult<Json<Value>> {
    let delta = match body.direction.as_str() {
        "up" => -1,
        "down" => 1,
        _ => return Err(AppError::bad_request("direction must be \"up\" or \"down\"")),
    };
    jobs::move_in_queue(&state, &id, delta).await.map_err(bad)?;
    Ok(Json(json!({"ok": true})))
}

/// The plate is clear: start the next queued job on this printer.
#[derive(Deserialize)]
pub struct StartBody {
    printer_id: String,
}

/// Start a queued print on a printer of its type.
pub async fn start_queued(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<StartBody>) -> ApiResult<Json<JobView>> {
    let job = jobs::start_queued(&state, &id, &body.printer_id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

/// Slice a queued print again for a printer of another type.
pub async fn reslice_queued(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<StartBody>) -> ApiResult<Json<JobView>> {
    let job = jobs::reslice_queued(&state, &id, &body.printer_id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn start_next(State(state): State<Shared>, Path(printer_id): Path<String>) -> ApiResult<Json<JobView>> {
    let job = jobs::start_next(&state, &printer_id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

#[derive(Deserialize)]
pub struct ImageQuery {
    u: String,
}

/// MakerWorld's picture hosts (https only).
pub fn is_makerworld_picture(url: &url::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| h.ends_with(".bblmw.com"))
}

/// Pictures larger than this aren't fetched (the resized ones are ~30 KB).
const MAX_PICTURE: usize = 8 * 1024 * 1024;

/// The picture format from the first bytes; `None` when it isn't one.
fn picture_kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes.get(..12)? {
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P'] => Some("image/webp"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, ..] => Some("image/jpeg"),
        _ => None,
    }
}

/// A model site's picture for a notification. MakerWorld's resizer only
/// answers in WebP, which iPhones don't show in notifications, so its
/// original is fetched instead.
pub async fn site_picture(url: &str) -> anyhow::Result<crate::notify::Picture> {
    let mut parsed = url::Url::parse(url)?;
    if parsed.scheme() != "https" {
        anyhow::bail!("not an https address");
    }
    if is_makerworld_picture(&parsed) {
        parsed.set_query(None);
    }
    let bytes = fetch_picture(parsed.as_str()).await?;
    let mime = picture_kind(&bytes).ok_or_else(|| anyhow::anyhow!("not a picture"))?;
    Ok(crate::notify::Picture { bytes: std::sync::Arc::new(bytes), mime })
}

/// Fetch a picture without following redirects (they could leave
/// MakerWorld's hosts) and without buffering more than `MAX_PICTURE`.
async fn fetch_picture(url: &str) -> anyhow::Result<Vec<u8>> {
    use futures::StreamExt;
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default()
    });
    let resp = client.get(url).send().await?.error_for_status()?;
    if resp.content_length().is_some_and(|n| n as usize > MAX_PICTURE) {
        anyhow::bail!("the picture is too large");
    }
    let mut body = Vec::new();
    let mut chunks = resp.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        body.extend_from_slice(&chunk?);
        if body.len() > MAX_PICTURE {
            anyhow::bail!("the picture is too large");
        }
    }
    Ok(body)
}

/// A MakerWorld picture, fetched by the server: the CDN hides pictures from
/// some browsers behind a bot check. Only MakerWorld's picture hosts; kept on
/// disk, trimmed hourly to 50 MB and 30 days unused (`jobs::trim_cache`).
pub async fn makerworld_image(State(state): State<Shared>, Query(q): Query<ImageQuery>) -> ApiResult<impl IntoResponse> {
    let url = url::Url::parse(&q.u).map_err(|_| AppError::bad_request("not a picture address"))?;
    if !is_makerworld_picture(&url) {
        return Err(AppError::bad_request("only MakerWorld pictures"));
    }
    let wanted = crate::sources::makerworld::thumb(q.u.clone());
    use sha2::Digest;
    let key = hex::encode(sha2::Sha256::digest(wanted.as_bytes()));
    let dir = jobs::image_cache_dir(&state);
    let path = dir.join(format!("{key}.img"));
    let cached = tokio::fs::read(&path).await.ok().filter(|b| picture_kind(b).is_some());
    let bytes = match cached {
        Some(b) => {
            // Mark it used, so trimming keeps it (`jobs::trim_cache`).
            let p = path.clone();
            let _ = tokio::task::spawn_blocking(move || std::fs::File::options().write(true).open(&p).and_then(|f| f.set_modified(std::time::SystemTime::now()))).await;
            b
        }
        None => {
            let b = fetch_picture(&wanted).await.map_err(|e| AppError::bad_gateway(format!("MakerWorld: {e:#}")))?;
            if picture_kind(&b).is_none() {
                return Err(AppError::bad_gateway("MakerWorld didn't send a picture"));
            }
            // Written whole, then renamed, so a parallel request never reads half.
            let _ = tokio::fs::create_dir_all(&dir).await;
            let tmp = dir.join(format!("{key}.{}.tmp", uuid::Uuid::new_v4().simple()));
            if tokio::fs::write(&tmp, &b).await.is_ok() && tokio::fs::rename(&tmp, &path).await.is_err() {
                let _ = tokio::fs::remove_file(&tmp).await;
            }
            b
        }
    };
    let kind = picture_kind(&bytes).unwrap_or("image/jpeg");
    Ok(([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "private, max-age=2592000")], bytes))
}

pub async fn thumbnail(
    State(state): State<Shared>,
    Path((id, plate)): Path<(String, u32)>,
) -> ApiResult<impl IntoResponse> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let file = job.input_file.ok_or_else(|| AppError::not_found("not downloaded yet"))?;
    let path = jobs::job_dir(&state, &id).join(file);
    let bytes = tokio::task::spawn_blocking(move || threemf::thumbnail(&path, plate))
        .await
        .map_err(|e| AppError::from(anyhow::anyhow!(e)))?
        .map_err(|e| AppError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "private, max-age=86400")], bytes))
}

/// A plate picture from an added model's file.
pub async fn extra_thumbnail(State(state): State<Shared>, Path((id, extra, plate)): Path<(String, String, u32)>) -> ApiResult<impl IntoResponse> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let extra = job.extras.iter().find(|e| e.id == extra).ok_or_else(|| AppError::not_found("that model isn't on this plate"))?;
    let path = jobs::job_dir(&state, &id).join("extras").join(&extra.input_file);
    let bytes = tokio::task::spawn_blocking(move || threemf::thumbnail(&path, plate))
        .await
        .map_err(|e| AppError::from(anyhow::anyhow!(e)))?
        .map_err(|e| AppError::not_found(format!("{e:#}")))?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "private, max-age=86400")], bytes))
}

/// One call from a link to a running print. Meant for Home Assistant and
/// iOS Shortcuts; poll `GET /api/jobs/{id}` for progress.
#[derive(Deserialize)]
pub struct QuickPrint {
    url: String,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    printer_id: Option<String>,
    #[serde(default)]
    plate: Option<u32>,
    #[serde(default)]
    settings: Option<String>,
    #[serde(default)]
    process_profile: Option<String>,
    #[serde(default)]
    filament_profile: Option<String>,
    #[serde(default)]
    print: Option<PrintOptions>,
    /// Copies (any file); scale and rotation (STL/OBJ/STEP only).
    #[serde(default, flatten)]
    transform: crate::models::MeshTransform,
    /// STL/OBJ/STEP: slice with the filament in this tray.
    #[serde(default)]
    tray_id: Option<i32>,
    /// One tray per project filament (-1 = none chosen).
    #[serde(default)]
    tray_ids: Option<Vec<i32>>,
    /// Build plate; the printer's default plate when omitted.
    #[serde(default)]
    bed_type: Option<String>,
    /// Objects (3MF ids) to print from the plate; all when omitted.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// A saved preset by name, e.g. "Draft".
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    tweaks: crate::models::SliceTweaks,
}

pub async fn quick_print(State(state): State<Shared>, Json(body): Json<QuickPrint>) -> ApiResult<Json<JobView>> {
    let printer = jobs::find_printer(&state, body.printer_id.as_deref()).await.map_err(bad)?;
    let tweaks = match body.preset.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(name) => {
            let presets = state.store.read(|d| d.slice_presets.clone()).await.unwrap_or_else(crate::models::default_presets);
            presets
                .into_iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .map(|p| p.tweaks)
                .ok_or_else(|| AppError::bad_request(format!("no preset called {name}")))?
        }
        None => body.tweaks,
    };
    let slice = SliceOptions {
        printer_id: Some(printer.id.clone()),
        plate: body.plate,
        settings: body.settings,
        process_profile: body.process_profile,
        filament_profile: body.filament_profile,
        transform: body.transform,
        tray_id: body.tray_id,
        tray_ids: body.tray_ids,
        bed_type: body.bed_type,
        objects: body.objects,
        tweaks,
        extras: vec![],
        also_plates: vec![],
    };
    let job = jobs::create(&state, &body.url, body.file_id.as_deref(), Some((slice, body.print.unwrap_or_default())))
        .await
        .map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

/// Upload a model file directly (multipart field `file`).
pub async fn upload(State(state): State<Shared>, mut multipart: axum::extract::Multipart) -> ApiResult<Json<JobView>> {
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::bad_request(e.to_string()))? {
        if field.name() != Some("file") {
            continue;
        }
        let name = field.file_name().unwrap_or("upload.3mf").to_string();
        let job = jobs::create_upload(&state, &name, field).await.map_err(bad)?;
        return Ok(Json(view(&state, job).await));
    }
    Err(AppError::bad_request("no file in the upload (use the form field \"file\")"))
}

// ------------------------------------------------------------------ models added to the plate

pub async fn add_extra(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<CreateBody>) -> ApiResult<Json<JobView>> {
    let job = jobs::add_extra(&state, &id, &body.url, body.file_id.as_deref()).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

#[derive(Deserialize)]
pub struct ExtraFromJob {
    job_id: String,
}

pub async fn add_extra_from_job(State(state): State<Shared>, Path(id): Path<String>, Json(body): Json<ExtraFromJob>) -> ApiResult<Json<JobView>> {
    let job = jobs::add_extra_from_job(&state, &id, &body.job_id).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

pub async fn upload_extra(State(state): State<Shared>, Path(id): Path<String>, mut multipart: axum::extract::Multipart) -> ApiResult<Json<JobView>> {
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::bad_request(e.to_string()))? {
        if field.name() != Some("file") {
            continue;
        }
        let name = field.file_name().unwrap_or("upload.3mf").to_string();
        let job = jobs::add_extra_upload(&state, &id, &name, field).await.map_err(bad)?;
        return Ok(Json(view(&state, job).await));
    }
    Err(AppError::bad_request("no file in the upload (use the form field \"file\")"))
}

pub async fn remove_extra(State(state): State<Shared>, Path((id, extra)): Path<(String, String)>) -> ApiResult<Json<JobView>> {
    let job = jobs::remove_extra(&state, &id, &extra).await.map_err(bad)?;
    Ok(Json(view(&state, job).await))
}

// ------------------------------------------------------------------ preview

#[derive(Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    plate: Option<u32>,
    #[serde(default)]
    printer_id: Option<String>,
    /// `input` or `sliced`; default: sliced when available.
    #[serde(default)]
    source: Option<String>,
}

/// Geometry for the 3D plate preview (cached per job, plate and printer).
#[derive(Deserialize)]
pub struct TraySuggestionQuery {
    printer_id: Option<String>,
}

/// Trays to suggest before slicing, one per filament of the file: same
/// material, closest colour, as when printing.
pub async fn tray_suggestion(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<TraySuggestionQuery>,
) -> ApiResult<Json<MappingSuggestion>> {
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let insp = job.inspection.ok_or_else(|| AppError::bad_request("file is not downloaded yet"))?;
    let printer = jobs::find_printer(&state, q.printer_id.as_deref()).await.map_err(bad)?;
    let status = state.printers.status(&printer.id).await;
    let filaments = suggestion_filaments(&insp);
    let slots = filaments.len() as u32;
    Ok(Json(mapping::suggest_with(&filaments, slots, &status, &[])))
}

/// The file's filaments as the mapping sees them; a bare mesh is one
/// filament of any material.
fn suggestion_filaments(insp: &crate::models::Inspection) -> Vec<crate::models::SlicedFilament> {
    if insp.kind == "mesh" || insp.filaments.is_empty() {
        return vec![crate::models::SlicedFilament { id: 1, kind: String::new(), color: String::new(), used_g: 1.0, used_m: 0.0 }];
    }
    insp.filaments
        .iter()
        .enumerate()
        .map(|(i, f)| crate::models::SlicedFilament { id: i as u32 + 1, kind: f.kind.clone(), color: f.color.clone(), used_g: 1.0, used_m: 0.0 })
        .collect()
}

pub async fn preview(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<PreviewQuery>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    let job = jobs::get_job(&state, &id).await.ok_or_else(|| AppError::not_found("job not found"))?;
    let dir = jobs::job_dir(&state, &id);
    let sliced_path = job.output_file.as_ref().map(|f| dir.join(f)).filter(|p| p.exists());
    let use_sliced = match q.source.as_deref() {
        Some("sliced") => true,
        Some("input") => false,
        _ => job.slice.is_some() && sliced_path.is_some(),
    };
    let (path, plate, kind) = if use_sliced {
        let slice = job.slice.as_ref().ok_or_else(|| AppError::bad_request("not sliced yet"))?;
        (sliced_path.ok_or_else(|| AppError::not_found("sliced file is missing"))?, slice.plate, "project")
    } else {
        let input = job.input_file.as_ref().map(|f| dir.join(f)).ok_or_else(|| AppError::not_found("not downloaded yet"))?;
        let insp = job.inspection.as_ref().ok_or_else(|| AppError::not_found("not downloaded yet"))?;
        let plate = q.plate.or_else(|| insp.plates.first().map(|p| p.index)).unwrap_or(1);
        (input, plate, if insp.kind == "mesh" { "mesh" } else { "project" })
    };

    // Bed of the printer this is for (used when the file doesn't say).
    let printer_id = q.printer_id.clone().or_else(|| job.slice.as_ref().map(|s| s.printer_id.clone()));
    let printer = jobs::find_printer(&state, printer_id.as_deref()).await.ok();
    let bed = match &printer {
        Some(p) => match state.slicer.profiles().await {
            Ok(index) => index.bed(&p.machine_profile).unwrap_or_else(|| crate::preview::Bed::for_model(&p.printer_model)),
            Err(_) => crate::preview::Bed::for_model(&p.printer_model),
        },
        None => crate::preview::Bed::rect(256.0, 256.0, 256.0),
    };

    let source = if use_sliced { "sliced" } else { "input" };
    let printer_key = printer.as_ref().map(|p| p.id.as_str()).unwrap_or("none");
    // Bump the version when the format or the simplifying changes (older caches are stale then).
    // A new slice replaces the file under the same name, so the name of its cache says which one it is.
    let stamp = match tokio::fs::metadata(&path).await.ok().and_then(|m| Some((m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(), m.len()))) {
        Some((at, len)) if use_sliced => format!("-{at}-{len}"),
        _ => String::new(),
    };
    let cache = dir.join(format!("preview-v4-{source}-{plate}-{printer_key}{stamp}.json"));
    let bytes = match tokio::fs::read(&cache).await {
        Ok(b) => b,
        Err(_) => {
            let built = tokio::task::spawn_blocking(move || {
                if kind == "mesh" {
                    crate::preview::mesh_preview(&path, bed)
                } else {
                    crate::preview::project_preview(&path, plate, bed)
                }
            })
            .await
            .map_err(|e| AppError::from(anyhow::anyhow!(e)))?
            .map_err(|e| {
                tracing::warn!(job = %id, plate, "preview failed: {e:#}");
                AppError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}"))
            })?;
            let b = serde_json::to_vec(&built).map_err(|e| AppError::from(anyhow::anyhow!(e)))?;
            if use_sliced {
                // Previews of earlier slices are of no more use.
                if let Ok(mut list) = tokio::fs::read_dir(&dir).await {
                    while let Ok(Some(f)) = list.next_entry().await {
                        if f.file_name().to_string_lossy().starts_with("preview-v4-sliced-") {
                            let _ = tokio::fs::remove_file(f.path()).await;
                        }
                    }
                }
            }
            let _ = tokio::fs::write(&cache, &b).await;
            b
        }
    };
    Ok(([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], bytes).into_response())
}

#[derive(Deserialize)]
pub struct SliceAllBody {
    #[serde(flatten)]
    options: SliceOptions,
    /// Plates to include; all when omitted.
    #[serde(default)]
    plates: Option<Vec<u32>>,
}

/// Slice every plate (or the listed ones) and queue them in order.
pub async fn slice_all(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<SliceAllBody>,
) -> ApiResult<Json<Vec<Job>>> {
    let jobs = jobs::slice_all(&state, &id, body.options, body.plates).await.map_err(bad)?;
    Ok(Json(jobs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_makerworld_pictures_and_real_ones() {
        let ok = |u: &str| is_makerworld_picture(&url::Url::parse(u).unwrap());
        assert!(ok("https://makerworld.bblmw.com/a.png"));
        assert!(ok("https://public-cdn.bblmw.com/avatar.png"));
        assert!(!ok("http://makerworld.bblmw.com/a.png"));
        assert!(!ok("https://xbblmw.com/a.png"));
        assert!(!ok("https://evil.com/makerworld.bblmw.com/a.png"));
        assert!(!ok("https://bblmw.com.evil.com/a.png"));
        assert_eq!(picture_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(picture_kind(b"<!DOCTYPE html><html>"), None);
    }
}
