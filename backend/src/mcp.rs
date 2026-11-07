//! MCP server (Model Context Protocol, Streamable HTTP transport) at `POST /api/mcp`.
//!
//! An AI assistant (Claude, ChatGPT, an IDE agent) can look up models, slice,
//! queue and start prints and read printer status. It signs in with an API key
//! like any other integration, and each tool is a call to the REST API with
//! that same key, so the checks are the ones the app's own requests go through.
//!
//! Stateless: no `Mcp-Session-Id`, every reply is plain JSON (no SSE stream),
//! which the spec allows. Starting a print needs `plate_is_clear: true`, since
//! a person has to have looked at the printer (the queue's rule too).

use crate::{auth::Principal, state::Shared};
use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine;
use serde_json::{Map, Value, json};
use tower::ServiceExt;

/// Newest first; anything else gets the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "AirSlice slices 3D models with OrcaSlicer and prints them on Bambu Lab printers on the local network. \
Typical flow: look_up_model (a MakerWorld, Printables or Thingiverse link) → create_job (downloads the file) → get_job until \
state is \"ready\" → slice_job → get_job until \"sliced\" (it shows the filament-to-tray suggestion and a cost) → \
print_job or queue_job. print_link does all of it in one call. Slicing takes from seconds to a few minutes. \
Never set plate_is_clear without the person saying the build plate is empty: a print started onto an old print can damage the printer.";

fn tools() -> Value {
    let id = |what: &str| json!({"type": "string", "description": what});
    let plate_clear = json!({"type": "boolean", "description": "The person confirmed the build plate is empty and the printer is ready. Ask them; never assume."});
    let ams_mapping = json!({"type": "array", "items": {"type": "integer"},
        "description": "One AMS tray per project filament slot: global tray id (AMS unit × 4 + slot, 0-based), 254 = external spool, -1 = unused. Omit to use the suggestion from get_job."});
    let read = json!({"readOnlyHint": true, "openWorldHint": false});
    Value::Array(vec![
        json!({
            "name": "list_printers", "title": "List printers",
            "description": "All printers with live status: state, progress, temperatures, loaded AMS trays (filament type, colour), problems (HMS), queue length, the current and last job.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": read,
        }),
        json!({
            "name": "get_printer", "title": "Printer status",
            "description": "One printer's live status, as in list_printers.",
            "inputSchema": {"type": "object", "properties": {"printer_id": id("Printer id from list_printers")}, "required": ["printer_id"]},
            "annotations": read,
        }),
        json!({
            "name": "camera_snapshot", "title": "Camera picture",
            "description": "A current picture from the printer's camera (JPEG). Use it to see a print's progress, or what's on the plate. It doesn't replace the person's confirmation that the plate is clear.",
            "inputSchema": {"type": "object", "properties": {"printer_id": id("Printer id from list_printers")}, "required": ["printer_id"]},
            "annotations": read,
        }),
        json!({
            "name": "control_printer", "title": "Control printer",
            "description": "Pause, resume or stop the running print, switch the chamber light, set print speed (speed_1 silent … speed_4 ludicrous) or ask for a fresh status report. Stop can't be undone: confirm with the person first.",
            "inputSchema": {"type": "object", "properties": {
                "printer_id": id("Printer id from list_printers"),
                "action": {"type": "string", "enum": ["pause", "resume", "stop", "light_on", "light_off", "refresh", "speed_1", "speed_2", "speed_3", "speed_4"]},
            }, "required": ["printer_id", "action"]},
            "annotations": {"destructiveHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "look_up_model", "title": "Look up a model",
            "description": "Read a model page (MakerWorld, Printables or Thingiverse link): title, description, the files or print profiles it has (their ids are file_id for create_job), plates, print time and weight where the site says.",
            "inputSchema": {"type": "object", "properties": {"url": id("Link to the model page")}, "required": ["url"]},
            "annotations": {"readOnlyHint": true, "openWorldHint": true},
        }),
        json!({
            "name": "list_collections", "title": "MakerWorld collections",
            "description": "The linked Bambu account's MakerWorld collections (own, liked, downloaded).",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true, "openWorldHint": true},
        }),
        json!({
            "name": "get_collection", "title": "Models in a collection",
            "description": "A page of models in one MakerWorld collection. Each model has an id; its link is https://makerworld.com/en/models/<id>.",
            "inputSchema": {"type": "object", "properties": {
                "collection_id": id("Collection id from list_collections"),
                "offset": {"type": "integer", "minimum": 0},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50},
            }, "required": ["collection_id"]},
            "annotations": {"readOnlyHint": true, "openWorldHint": true},
        }),
        json!({
            "name": "create_job", "title": "Download a model",
            "description": "Start a job from a model link: AirSlice downloads the file and reads its plates. Returns the job; poll get_job until state is \"ready\" (or \"failed\"). Without file_id the best file or profile for the default printer is picked.",
            "inputSchema": {"type": "object", "properties": {
                "url": id("Link to the model page"),
                "file_id": id("A file or print profile id from look_up_model"),
            }, "required": ["url"]},
            "annotations": {"openWorldHint": true},
        }),
        json!({
            "name": "slice_job", "title": "Slice",
            "description": "Slice a ready job with OrcaSlicer for a printer. Returns at once with state \"slicing\"; poll get_job until \"sliced\", which brings print time, weight, the tray suggestion and a cost estimate.",
            "inputSchema": {"type": "object", "properties": {
                "job_id": id("Job id"),
                "printer_id": id("Printer id; the default printer when omitted"),
                "plate": {"type": "integer", "minimum": 1, "description": "Plate number of a 3MF project (inspection.plates[].index); the first when omitted"},
                "settings": {"type": "string", "enum": ["auto", "project", "printer"], "description": "auto: the project's own settings when it was made for this printer, else the printer's profiles"},
                "bed_type": {"type": "string", "description": "Build plate, e.g. \"Textured PEI Plate\" (one of the printer's plates); the printer's default when omitted"},
                "copies": {"type": "integer", "minimum": 1},
                "scale_percent": {"type": "number", "description": "Meshes only (STL/OBJ/STEP)"},
                "rotate_z": {"type": "number", "description": "Meshes only: rotation in degrees"},
                "objects": {"type": "array", "items": {"type": "string"}, "description": "Object ids on the plate to print; all when omitted"},
                "tray_ids": {"type": "array", "items": {"type": "integer"}, "description": "One tray per project filament to slice with its filament and colour (-1 = none chosen)"},
                "tweaks": {"type": "object", "description": "Changes to the settings", "properties": {
                    "layer_height": {"type": "number"}, "walls": {"type": "integer"},
                    "infill": {"type": "integer", "description": "Sparse infill, percent"},
                    "supports": {"type": "string", "enum": ["off", "normal", "tree"]},
                    "brim": {"type": "boolean"},
                }},
            }, "required": ["job_id"]},
            "annotations": {"openWorldHint": false},
        }),
        json!({
            "name": "get_job", "title": "Job",
            "description": "A job: state (downloading, ready, slicing, sliced, queued, uploading, waiting, started, failed …), plates, slice result, the tray suggestion and loaded trays, cost estimate, live progress while printing and the last log lines.",
            "inputSchema": {"type": "object", "properties": {"job_id": id("Job id")}, "required": ["job_id"]},
            "annotations": read,
        }),
        json!({
            "name": "list_jobs", "title": "Recent jobs",
            "description": "Recent jobs, newest first, in short form. get_job has the details.",
            "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 20"}}},
            "annotations": read,
        }),
        json!({
            "name": "print_job", "title": "Start print",
            "description": "Send a sliced job to its printer and start it. An offline printer gets it when it comes back (state \"waiting\"). Only with the person's word that the plate is clear.",
            "inputSchema": {"type": "object", "properties": {
                "job_id": id("Job id"),
                "plate_is_clear": plate_clear,
                "ams_mapping": ams_mapping,
                "timelapse": {"type": "boolean"},
                "bed_leveling": {"type": "boolean", "description": "Default true"},
            }, "required": ["job_id", "plate_is_clear"]},
            "annotations": {"destructiveHint": false, "openWorldHint": false},
        }),
        json!({
            "name": "queue_job", "title": "Add to queue",
            "description": "Put a sliced job in the queue of its printer type, to start later with start_next (or from the app).",
            "inputSchema": {"type": "object", "properties": {"job_id": id("Job id"), "ams_mapping": ams_mapping}, "required": ["job_id"]},
            "annotations": {"openWorldHint": false},
        }),
        json!({
            "name": "get_queue", "title": "Queue",
            "description": "Queued prints, per printer type, with position, print time, weight and whether the loaded filament matches.",
            "inputSchema": {"type": "object", "properties": {"printer_id": id("Only this printer's queue")}},
            "annotations": read,
        }),
        json!({
            "name": "start_next", "title": "Start next in queue",
            "description": "Start the next queued print on this printer. Only with the person's word that the plate is clear.",
            "inputSchema": {"type": "object", "properties": {"printer_id": id("Printer id"), "plate_is_clear": plate_clear}, "required": ["printer_id", "plate_is_clear"]},
            "annotations": {"openWorldHint": false},
        }),
        json!({
            "name": "print_link", "title": "Print a link",
            "description": "Everything in one call: download a model, slice it and start the print as soon as it's sliced, with the suggested trays. Returns the job; poll get_job for progress. Only with the person's word that the plate is clear.",
            "inputSchema": {"type": "object", "properties": {
                "url": id("Link to the model page"),
                "plate_is_clear": plate_clear,
                "printer_id": id("Printer id; the default printer when omitted"),
                "file_id": id("A file or print profile id from look_up_model"),
                "plate": {"type": "integer", "minimum": 1},
                "preset": id("A saved slice preset by name, e.g. \"Draft\" (list_presets)"),
                "copies": {"type": "integer", "minimum": 1},
                "bed_type": id("Build plate; the printer's default when omitted"),
            }, "required": ["url", "plate_is_clear"]},
            "annotations": {"openWorldHint": true},
        }),
        json!({
            "name": "list_presets", "title": "Slice presets",
            "description": "The saved slice presets (name + settings tweaks), for print_link's preset or slice_job's tweaks.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": read,
        }),
        json!({
            "name": "list_prints", "title": "Print history",
            "description": "Finished, failed and cancelled prints, newest first: title, printer, times, filament used, cost, also prints not started by AirSlice (external).",
            "inputSchema": {"type": "object", "properties": {
                "printer_id": id("Only this printer"),
                "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Default 20"},
            }},
            "annotations": read,
        }),
    ])
}

/// A tool call as a request to the REST API (relative to /api).
#[derive(Debug, PartialEq)]
struct ApiCall {
    method: Method,
    path: String,
    body: Option<Value>,
}

#[derive(Debug, PartialEq)]
enum Plan {
    Api(ApiCall),
    /// The person hasn't confirmed something the tool needs.
    Refuse(String),
}

/// Ids go into the request path: only what AirSlice's own ids look like.
fn path_id(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    let v = match args.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err(format!("{key} is required")),
    };
    if v.is_empty() || v.len() > 100 || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("{key} {v:?} isn't a valid id"));
    }
    Ok(v)
}

fn get_u64(args: &Map<String, Value>, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

/// The arguments minus the given keys, as a request body.
fn body_without(args: &Map<String, Value>, drop: &[&str]) -> Value {
    Value::Object(args.iter().filter(|(k, _)| !drop.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn plate_confirmed(args: &Map<String, Value>) -> Option<Plan> {
    (args.get("plate_is_clear") != Some(&Value::Bool(true))).then(|| {
        Plan::Refuse(
            "Not started: ask the person whether the build plate is empty and the printer is ready, then call again with plate_is_clear: true.".into(),
        )
    })
}

fn plan(name: &str, args: &Map<String, Value>) -> Result<Plan, String> {
    let call = |method: Method, path: String, body: Option<Value>| Ok(Plan::Api(ApiCall { method, path, body }));
    match name {
        "list_printers" => call(Method::GET, "/printers".into(), None),
        "get_printer" => call(Method::GET, format!("/printers/{}", path_id(args, "printer_id")?), None),
        "camera_snapshot" => call(Method::GET, format!("/printers/{}/camera.jpg", path_id(args, "printer_id")?), None),
        "control_printer" => {
            let action = args.get("action").and_then(Value::as_str).ok_or("action is required")?;
            call(Method::POST, format!("/printers/{}/control", path_id(args, "printer_id")?), Some(json!({"action": action})))
        }
        "look_up_model" => call(Method::POST, "/resolve".into(), Some(body_without(args, &[]))),
        "list_collections" => call(Method::GET, "/makerworld/collections".into(), None),
        "get_collection" => call(
            Method::GET,
            format!(
                "/makerworld/collections/{}?offset={}&limit={}",
                path_id(args, "collection_id")?,
                get_u64(args, "offset").unwrap_or(0),
                get_u64(args, "limit").unwrap_or(24).clamp(1, 50)
            ),
            None,
        ),
        "create_job" => call(Method::POST, "/jobs".into(), Some(body_without(args, &[]))),
        "slice_job" => call(Method::POST, format!("/jobs/{}/slice", path_id(args, "job_id")?), Some(body_without(args, &["job_id"]))),
        "get_job" => call(Method::GET, format!("/jobs/{}", path_id(args, "job_id")?), None),
        "list_jobs" => call(Method::GET, format!("/jobs?limit={}", get_u64(args, "limit").unwrap_or(20).clamp(1, 200)), None),
        "print_job" => {
            let path = format!("/jobs/{}/print", path_id(args, "job_id")?);
            if let Some(refusal) = plate_confirmed(args) {
                return Ok(refusal);
            }
            call(Method::POST, path, Some(body_without(args, &["job_id", "plate_is_clear"])))
        }
        "queue_job" => call(Method::POST, format!("/jobs/{}/queue", path_id(args, "job_id")?), Some(body_without(args, &["job_id"]))),
        "get_queue" => match args.contains_key("printer_id") {
            true => call(Method::GET, format!("/queue?printer_id={}", path_id(args, "printer_id")?), None),
            false => call(Method::GET, "/queue".into(), None),
        },
        "start_next" => {
            let path = format!("/printers/{}/start-next", path_id(args, "printer_id")?);
            if let Some(refusal) = plate_confirmed(args) {
                return Ok(refusal);
            }
            call(Method::POST, path, None)
        }
        "print_link" => {
            if let Some(refusal) = plate_confirmed(args) {
                return Ok(refusal);
            }
            call(Method::POST, "/print".into(), Some(body_without(args, &["plate_is_clear"])))
        }
        "list_presets" => call(Method::GET, "/settings/presets".into(), None),
        "list_prints" => {
            let limit = get_u64(args, "limit").unwrap_or(20).clamp(1, 500);
            match args.contains_key("printer_id") {
                true => call(Method::GET, format!("/prints?limit={limit}&printer_id={}", path_id(args, "printer_id")?), None),
                false => call(Method::GET, format!("/prints?limit={limit}"), None),
            }
        }
        other => Err(format!("unknown tool {other:?}")),
    }
}

/// Smaller answers for a language model: jobs in short form, a job's log cut
/// to its end.
fn shape(tool: &str, mut v: Value) -> Value {
    fn trim_job(job: &mut Value) {
        if let Some(obj) = job.as_object_mut() {
            for k in ["input_file", "output_file", "auto_slice", "auto_print", "model_cover"] {
                obj.remove(k);
            }
            if let Some(Value::Array(log)) = obj.get_mut("log")
                && log.len() > 15
            {
                log.drain(..log.len() - 15);
            }
        }
    }
    match tool {
        "list_jobs" => {
            if let Value::Array(jobs) = &mut v {
                for j in jobs.iter_mut() {
                    let pick = |k: &str| j.get(k).cloned().unwrap_or(Value::Null);
                    *j = json!({
                        "id": pick("id"), "state": pick("state"), "title": pick("model_title"),
                        "file": j.pointer("/file/name").cloned().unwrap_or(Value::Null),
                        "plate_label": pick("plate_label"), "created_at": pick("created_at"),
                        "error": pick("error"), "favorite": pick("favorite"),
                        "outcome": j.pointer("/print/outcome").cloned().unwrap_or(Value::Null),
                    });
                }
            }
        }
        "get_job" | "create_job" | "slice_job" | "print_job" | "queue_job" | "start_next" | "print_link" => trim_job(&mut v),
        _ => {}
    }
    v
}

fn text(s: impl Into<String>, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}], "isError": is_error})
}

async fn call_tool(state: &Shared, auth: &HeaderMap, name: &str, args: &Map<String, Value>) -> Value {
    let plan = match plan(name, args) {
        Ok(Plan::Api(c)) => c,
        Ok(Plan::Refuse(why)) => return text(why, true),
        Err(e) => return text(e, true),
    };
    let mut req = Request::builder().method(plan.method).uri(&plan.path);
    for h in [header::AUTHORIZATION.as_str(), "x-api-key"] {
        if let Some(v) = auth.get(h) {
            req = req.header(h, v);
        }
    }
    let body = match plan.body {
        Some(b) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let Ok(req) = req.body(body) else { return text("couldn't build the request", true) };
    let api = crate::api::router(state.clone()).with_state(state.clone());
    let res = match api.oneshot(req).await {
        Ok(r) => r,
        Err(e) => return text(format!("{e}"), true),
    };
    let status = res.status();
    let is_jpeg = res.headers().get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"image/jpeg"));
    let bytes = match axum::body::to_bytes(res.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return text(format!("reading the answer failed: {e}"), true),
    };
    if !status.is_success() {
        let msg = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        return text(format!("{msg} (HTTP {})", status.as_u16()), true);
    }
    if is_jpeg {
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return json!({"content": [{"type": "image", "data": data, "mimeType": "image/jpeg"}], "isError": false});
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => {
            let v = shape(name, v);
            let mut out = text(v.to_string(), false);
            // structuredContent must be an object.
            if v.is_object() {
                out["structuredContent"] = v;
            }
            out
        }
        Err(_) => text(String::from_utf8_lossy(&bytes).into_owned(), false),
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One JSON-RPC message; `None` for notifications, which get no answer.
async fn handle_message(state: &Shared, auth: &HeaderMap, msg: Value) -> Option<Value> {
    let Some(obj) = msg.as_object() else { return Some(rpc_error(Value::Null, -32600, "not a JSON-RPC message")) };
    let method = obj.get("method").and_then(Value::as_str);
    // Notifications (no id) and the client's replies to us (no method).
    let id = obj.get("id").cloned()?;
    let method = method?;
    let params = obj.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "airslice", "title": "AirSlice", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "params.name is required"));
            };
            if !tools().as_array().is_some_and(|t| t.iter().any(|t| t["name"] == name)) {
                return Some(rpc_error(id, -32602, &format!("unknown tool {name:?}")));
            }
            let args = params.get("arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            call_tool(state, auth, name, &args).await
        }
        "resources/list" => json!({"resources": []}),
        "prompts/list" => json!({"prompts": []}),
        _ => return Some(rpc_error(id, -32601, &format!("method {method:?} not found"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The browser's Origin, when sent, must be this server (DNS rebinding).
fn origin_ok(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(',').next().unwrap_or("").trim().to_ascii_lowercase());
    let origin_host = url::Url::parse(origin).ok().and_then(|u| {
        u.host_str().map(|h| match u.port() {
            Some(p) => format!("{h}:{p}"),
            None => h.to_string(),
        })
    });
    matches!((origin_host, host), (Some(o), Some(h)) if o.eq_ignore_ascii_case(&h))
}

pub async fn post(
    State(state): State<Shared>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !origin_ok(&headers) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "requests from other sites aren't allowed"}))).into_response();
    }
    // Only API keys: they're what an assistant is given, and a browser's
    // cookie can't be talked into calling tools from another page.
    if !matches!(principal, Principal::ApiKey { .. }) {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "the MCP server needs an API key (Settings → API keys)"}))).into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Json(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))).into_response(),
    };
    let answer = match msg {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in batch {
                if let Some(a) = handle_message(&state, &headers, m).await {
                    out.push(a);
                }
            }
            (!out.is_empty()).then_some(Value::Array(out))
        }
        m => handle_message(&state, &headers, m).await,
    };
    match answer {
        Some(a) => Json(a).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// No server-to-client stream and no sessions.
pub async fn other() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn every_tool_has_a_plan() {
        for t in tools().as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            let required: Vec<&str> = t["inputSchema"]["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut a = Map::new();
            for k in required {
                a.insert(k.into(), if k == "plate_is_clear" { Value::Bool(true) } else { json!("abc") });
            }
            if name == "control_printer" {
                a.insert("action".into(), json!("pause"));
            }
            assert!(matches!(plan(name, &a), Ok(Plan::Api(_))), "{name}");
        }
    }

    #[test]
    fn printing_needs_a_clear_plate() {
        for (tool, a) in [
            ("print_job", json!({"job_id": "j1"})),
            ("print_job", json!({"job_id": "j1", "plate_is_clear": false})),
            ("start_next", json!({"printer_id": "p1", "plate_is_clear": "yes"})),
            ("print_link", json!({"url": "https://makerworld.com/en/models/1"})),
        ] {
            assert!(matches!(plan(tool, &args(a)), Ok(Plan::Refuse(_))), "{tool}");
        }
        let Ok(Plan::Api(c)) = plan("print_job", &args(json!({"job_id": "j1", "plate_is_clear": true, "timelapse": true}))) else { panic!() };
        assert_eq!(c.path, "/jobs/j1/print");
        assert_eq!(c.body, Some(json!({"timelapse": true})));
    }

    #[test]
    fn ids_cant_escape_the_path() {
        for bad in ["../keys", "a/b", "a?x=1", "", "a b", "a%2f"] {
            assert!(plan("get_job", &args(json!({"job_id": bad}))).is_err(), "{bad}");
        }
        assert!(plan("get_job", &args(json!({}))).is_err());
        let Ok(Plan::Api(c)) = plan("get_job", &args(json!({"job_id": "4f2c-9a_1"}))) else { panic!() };
        assert_eq!(c.path, "/jobs/4f2c-9a_1");
    }

    #[test]
    fn list_jobs_is_short() {
        let v = shape("list_jobs", json!([{"id": "a", "state": "sliced", "model_title": "Benchy", "file": {"name": "b.3mf"}, "log": ["x"], "inspection": {}}]));
        assert_eq!(v[0]["title"], "Benchy");
        assert_eq!(v[0]["file"], "b.3mf");
        assert!(v[0].get("log").is_none() && v[0].get("inspection").is_none());
        let log: Vec<Value> = (0..40).map(|i| json!(i)).collect();
        let v = shape("get_job", json!({"id": "a", "log": log, "input_file": "/data/x"}));
        assert_eq!(v["log"].as_array().unwrap().len(), 15);
        assert_eq!(v["log"][14], 39);
        assert!(v.get("input_file").is_none());
    }

    #[test]
    fn origin_must_match_host() {
        let mut h = HeaderMap::new();
        assert!(origin_ok(&h));
        h.insert(header::HOST, "airslice.local:8080".parse().unwrap());
        h.insert(header::ORIGIN, "http://airslice.local:8080".parse().unwrap());
        assert!(origin_ok(&h));
        h.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!origin_ok(&h));
    }
}
