use crate::{
    bambu_cloud::{self, LoginStep},
    error::{ApiResult, AppError},
    models::{ConnectionSettings, CostSettings, NotifySettings, SpoolmanSettings, StorageSettings, now},
    slicer::{MachineProfile, ProfileChoices},
    state::Shared,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn health() -> Json<Value> {
    Json(json!({"ok": true}))
}

pub async fn info(State(state): State<Shared>) -> Json<Value> {
    let profiles = state.slicer.profiles().await;
    let (bambu_linked, bambu_email, region) =
        state.store.read(|d| (d.bambu.token.is_some(), d.bambu.email.clone(), d.bambu.region.clone())).await;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "slicer": {
            "available": state.slicer.available(),
            "path": state.slicer.binary(),
            "version": state.slicer.version().await,
            "profiles_loaded": profiles.is_ok(),
            "profiles_error": profiles.err().map(|e| format!("{e:#}")),
        },
        "bambu": { "linked": bambu_linked, "email": bambu_email, "region": region },
        "base_path": state.cfg.base_path,
    }))
}

/// Settings → About → Copy diagnostics: plain text, secrets taken out.
pub async fn diagnostics(State(state): State<Shared>) -> ([(axum::http::HeaderName, &'static str); 1], String) {
    ([(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")], crate::diagnostics::report(&state).await)
}

pub async fn machines(State(state): State<Shared>) -> ApiResult<Json<Vec<MachineProfile>>> {
    let index = state.slicer.profiles().await.map_err(|e| AppError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    Ok(Json(index.machines()))
}

#[derive(Deserialize)]
pub struct ProfilesQuery {
    machine: String,
}

pub async fn profiles(State(state): State<Shared>, Query(q): Query<ProfilesQuery>) -> ApiResult<Json<ProfileChoices>> {
    let index = state.slicer.profiles().await.map_err(|e| AppError::new(axum::http::StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")))?;
    if !index.has_machine(&q.machine) {
        return Err(AppError::not_found("unknown printer profile"));
    }
    Ok(Json(index.choices_for(&q.machine)))
}

// ------------------------------------------------------------------ Bambu account

fn china(region: Option<&str>) -> bool {
    region == Some("china")
}

#[derive(Deserialize)]
pub struct LoginBody {
    email: String,
    password: String,
    #[serde(default)]
    region: Option<String>,
}

pub async fn bambu_login(State(state): State<Shared>, Json(body): Json<LoginBody>) -> ApiResult<Json<Value>> {
    let step = bambu_cloud::login(&state.http, china(body.region.as_deref()), body.email.trim(), &body.password)
        .await
        .map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    let region = body.region.unwrap_or_else(|| "global".into());
    match step {
        LoginStep::Done { token } => {
            save_token(&state, token, Some(body.email.trim().to_string()), region).await?;
            Ok(Json(json!({"status": "done"})))
        }
        LoginStep::EmailCode => Ok(Json(json!({"status": "email_code"}))),
        LoginStep::Totp => Ok(Json(json!({
            "status": "totp",
            "message": "Accounts with an authenticator app can't sign in here yet. Paste an access token instead."
        }))),
    }
}

#[derive(Deserialize)]
pub struct VerifyBody {
    email: String,
    code: String,
    #[serde(default)]
    region: Option<String>,
}

pub async fn bambu_verify(State(state): State<Shared>, Json(body): Json<VerifyBody>) -> ApiResult<Json<Value>> {
    let token = bambu_cloud::verify_email_code(&state.http, china(body.region.as_deref()), body.email.trim(), &body.code)
        .await
        .map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    save_token(&state, token, Some(body.email.trim().to_string()), body.region.unwrap_or_else(|| "global".into())).await?;
    Ok(Json(json!({"status": "done"})))
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
    #[serde(default)]
    region: Option<String>,
}

pub async fn bambu_token(State(state): State<Shared>, Json(body): Json<TokenBody>) -> ApiResult<Json<Value>> {
    let token = body.token.trim().trim_start_matches("Bearer ").to_string();
    if token.len() < 20 {
        return Err(AppError::bad_request("that doesn't look like an access token"));
    }
    let region = body.region.unwrap_or_else(|| "global".into());
    match bambu_cloud::check_token(&state.http, china(Some(&region)), &token).await {
        Ok(false) => return Err(AppError::bad_request("Bambu rejected this token")),
        Ok(true) => {}
        Err(e) => tracing::warn!("could not verify token, saving anyway: {e:#}"),
    }
    save_token(&state, token, None, region).await?;
    Ok(Json(json!({"status": "done"})))
}

async fn save_token(state: &Shared, token: String, email: Option<String>, region: String) -> ApiResult<()> {
    state
        .store
        .update(|d| {
            d.bambu.token = Some(token);
            if email.is_some() {
                d.bambu.email = email;
            }
            d.bambu.region = region;
            d.bambu.updated_at = Some(now());
        })
        .await?;
    Ok(())
}

pub async fn bambu_status(State(state): State<Shared>) -> Json<Value> {
    let acct = state.store.read(|d| d.bambu.clone()).await;
    let valid = match &acct.token {
        Some(t) => bambu_cloud::check_token(&state.http, acct.region == "china", t).await.ok(),
        None => None,
    };
    Json(json!({
        "linked": acct.token.is_some(),
        "valid": valid,
        "email": acct.email,
        "region": acct.region,
        "updated_at": acct.updated_at,
    }))
}

pub async fn bambu_unlink(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    state
        .store
        .update(|d| {
            d.bambu.token = None;
            d.bambu.updated_at = Some(now());
        })
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// Retries and waiting for offline printers.
pub async fn connection(State(state): State<Shared>) -> Json<ConnectionSettings> {
    Json(state.printers.settings())
}

pub async fn set_connection(State(state): State<Shared>, Json(body): Json<ConnectionSettings>) -> ApiResult<Json<ConnectionSettings>> {
    body.validate().map_err(AppError::bad_request)?;
    state.store.update(|d| d.connection = body).await?;
    state.printers.set_settings(body);
    Ok(Json(body))
}

/// Notification settings; tokens never leave the server.
fn notify_view(s: &NotifySettings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or_default();
    if let Some(m) = v.as_object_mut() {
        m.remove("token");
        m.insert("has_token".into(), json!(!s.token.is_empty()));
        m.insert("in_addon".into(), json!(std::env::var("SUPERVISOR_TOKEN").is_ok_and(|t| !t.is_empty())));
    }
    if let Some(list) = v.get_mut("targets").and_then(|t| t.as_array_mut()) {
        for (item, t) in list.iter_mut().zip(&s.targets) {
            if let Some(m) = item.as_object_mut() {
                m.remove("telegram_token");
                m.remove("ntfy_token");
                m.remove("push_p256dh");
                m.remove("push_auth");
                m.insert("has_telegram_token".into(), json!(!t.telegram_token.is_empty()));
                m.insert("has_ntfy_token".into(), json!(!t.ntfy_token.is_empty()));
            }
        }
    }
    v
}

pub async fn notify(State(state): State<Shared>) -> Json<Value> {
    Json(notify_view(&state.store.read(|d| d.notify.clone()).await))
}

/// Tokens the browser left empty are the stored ones (it never sees them).
fn keep_secrets(body: &mut NotifySettings, old: &NotifySettings) {
    if body.token.trim().is_empty() && body.ha_url.trim() == old.ha_url.trim() {
        body.token = old.token.clone();
    }
    for t in &mut body.targets {
        let Some(o) = old.targets.iter().find(|o| o.id == t.id) else { continue };
        if t.telegram_token.trim().is_empty() {
            t.telegram_token = o.telegram_token.clone();
        }
        if t.ntfy_token.trim().is_empty() && t.ntfy_url.trim() == o.ntfy_url.trim() {
            t.ntfy_token = o.ntfy_token.clone();
        }
        // Only the browser itself sets these, by subscribing.
        t.push_endpoint = o.push_endpoint.clone();
        t.push_p256dh = o.push_p256dh.clone();
        t.push_auth = o.push_auth.clone();
    }
    // A new entry can't bring its own.
    for t in body.targets.iter_mut().filter(|t| !old.targets.iter().any(|o| o.id == t.id)) {
        t.push_endpoint.clear();
        t.push_p256dh.clear();
        t.push_auth.clear();
    }
}

/// Tidy a notification and say what is missing for it to work.
fn check_target(t: &mut crate::models::NotifyTarget) -> Result<(), String> {
    t.name = t.name.trim().chars().take(40).collect();
    t.service = t.service.trim().trim_start_matches("notify.").to_string();
    t.ntfy_url = t.ntfy_url.trim().trim_end_matches('/').to_string();
    t.telegram_chat = t.telegram_chat.trim().to_string();
    if !matches!(t.language.as_str(), "en" | "sv") {
        t.language.clear();
    }
    t.channel = match t.channel.trim() {
        "telegram" | "ntfy" | "webpush" => t.channel.trim().to_string(),
        _ => "ha".into(),
    };
    let name = if t.name.is_empty() { "a notification".to_string() } else { t.name.clone() };
    match t.channel.as_str() {
        "telegram" if t.telegram_token.trim().is_empty() || t.telegram_chat.is_empty() => Err(format!("{name}: enter the Telegram bot token and chat id")),
        "ntfy" if !t.ntfy_url.starts_with("http") => Err(format!("{name}: enter the ntfy topic address, e.g. https://ntfy.sh/my-printer")),
        "webpush" if !crate::webpush::is_endpoint(&t.push_endpoint) => Err(format!("{name}: turn notifications on from that browser")),
        "ha" if t.service.is_empty() => Err(format!("{name}: choose which notify service to use")),
        "ha" if !crate::notify::is_object_id(&t.service) => Err(format!("{name}: {:?} isn't a notify service name", t.service)),
        _ => Ok(()),
    }
}

/// Save; an empty or missing token keeps the stored one.
pub async fn set_notify(State(state): State<Shared>, Json(mut body): Json<NotifySettings>) -> ApiResult<Json<Value>> {
    let old = state.store.read(|d| d.notify.clone()).await;
    keep_secrets(&mut body, &old);
    body.ha_url = body.ha_url.trim().trim_end_matches('/').to_string();
    let mut used = std::collections::HashSet::new();
    for (i, t) in body.targets.iter_mut().enumerate() {
        if t.id.trim().is_empty() || !used.insert(t.id.clone()) {
            t.id = format!("{}-{i}", crate::models::now());
            used.insert(t.id.clone());
        }
        let problem = check_target(t);
        if body.enabled && t.enabled {
            problem.map_err(AppError::bad_request)?;
        }
    }
    let saved = body.clone();
    state.store.update(move |d| d.notify = body).await?;
    Ok(Json(notify_view(&saved)))
}

/// Notify services, using the settings in the body (so the list works
/// before saving).
pub async fn notify_services(State(state): State<Shared>, Json(mut body): Json<NotifySettings>) -> ApiResult<Json<Vec<String>>> {
    // The stored token only goes to the stored address.
    let old = state.store.read(|d| d.notify.clone()).await;
    keep_secrets(&mut body, &old);
    crate::notify::services(&state, &body).await.map(Json).map_err(|e| AppError::bad_gateway(format!("{e:#}")))
}

#[derive(Deserialize)]
pub struct TestQuery {
    target: String,
}

/// Send a test to one notification, with the settings on screen (saved or not).
pub async fn notify_test(
    State(state): State<Shared>,
    Query(q): Query<TestQuery>,
    Json(mut body): Json<NotifySettings>,
) -> ApiResult<Json<Value>> {
    let old = state.store.read(|d| d.notify.clone()).await;
    keep_secrets(&mut body, &old);
    let mut target = body.targets.iter().find(|t| t.id == q.target).cloned().ok_or_else(|| AppError::bad_request("no such notification"))?;
    check_target(&mut target).map_err(AppError::bad_request)?;
    let lang = crate::notify::language(&state, &body, &target).await;
    crate::notify::send_now(&state, &body, &target, "AirSlice", &crate::i18n::t(lang, "Notifications work.", &[]))
        .await
        .map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

/// The key a browser subscribes with.
pub async fn webpush_key(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    let key = crate::webpush::public_key(&state).await.map_err(|e| AppError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(Json(json!({ "public_key": key })))
}

#[derive(Deserialize)]
pub struct PushKeys {
    p256dh: String,
    auth: String,
}

/// A browser's `PushSubscription.toJSON()`, plus a name for it.
#[derive(Deserialize)]
pub struct PushSubscribe {
    endpoint: String,
    keys: PushKeys,
    #[serde(default)]
    name: String,
    #[serde(default)]
    language: String,
}

/// This browser takes notifications: add it (or renew its keys).
pub async fn webpush_subscribe(State(state): State<Shared>, Json(b): Json<PushSubscribe>) -> ApiResult<Json<Value>> {
    let endpoint = b.endpoint.trim().to_string();
    if !crate::webpush::is_endpoint(&endpoint) {
        return Err(AppError::bad_request("that isn't a push address AirSlice can use"));
    }
    let (p256dh, auth) = (b.keys.p256dh.trim().to_string(), b.keys.auth.trim().to_string());
    let keys_ok = crate::webpush::b64_decode(&p256dh).is_ok_and(|k| k.len() == 65) && crate::webpush::b64_decode(&auth).is_ok_and(|k| k.len() == 16);
    if !keys_ok {
        return Err(AppError::bad_request("the browser's keys look wrong"));
    }
    let name: String = b.name.trim().chars().take(40).collect();
    let language = if matches!(b.language.as_str(), "en" | "sv") { b.language.clone() } else { String::new() };
    let saved = state
        .store
        .update(move |d| {
            let n = &mut d.notify;
            match n.targets.iter_mut().find(|t| t.channel == "webpush" && t.push_endpoint == endpoint) {
                Some(t) => {
                    t.push_p256dh = p256dh;
                    t.push_auth = auth;
                    t.enabled = true;
                }
                None => {
                    if n.targets.is_empty() {
                        n.enabled = true;
                    }
                    let id = format!("push-{}", crate::auth::random_token().chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect::<String>());
                    n.targets.push(crate::models::NotifyTarget {
                        id,
                        name: if name.is_empty() { "This browser".into() } else { name },
                        channel: "webpush".into(),
                        push_endpoint: endpoint,
                        push_p256dh: p256dh,
                        push_auth: auth,
                        language,
                        ..Default::default()
                    });
                }
            }
            n.clone()
        })
        .await?;
    Ok(Json(notify_view(&saved)))
}

/// A notification's picture (no login: the browser fetches it on its own,
/// by an unguessable id that expires after an hour).
pub async fn notify_picture(Path(id): Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match crate::webpush::picture(&id) {
        Some((bytes, mime)) => ([(axum::http::header::CONTENT_TYPE, mime), (axum::http::header::CACHE_CONTROL, "private, max-age=3600")], bytes.to_vec()).into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

/// Saved slice settings (the defaults until someone changes them).
pub async fn presets(State(state): State<Shared>) -> Json<Vec<crate::models::SlicePreset>> {
    Json(state.store.read(|d| d.slice_presets.clone()).await.unwrap_or_else(crate::models::default_presets))
}

pub async fn set_presets(State(state): State<Shared>, Json(list): Json<Vec<crate::models::SlicePreset>>) -> ApiResult<Json<Vec<crate::models::SlicePreset>>> {
    let mut clean = Vec::new();
    for mut p in list {
        p.name = p.name.trim().chars().take(40).collect();
        if p.name.is_empty() {
            return Err(AppError::bad_request("give the preset a name"));
        }
        if p.tweaks.is_empty() {
            return Err(AppError::bad_request(format!("{} changes nothing", p.name)));
        }
        p.tweaks.validate().map_err(AppError::bad_request)?;
        if clean.iter().any(|c: &crate::models::SlicePreset| c.name.eq_ignore_ascii_case(&p.name)) {
            return Err(AppError::bad_request(format!("there are two presets called {}", p.name)));
        }
        clean.push(p);
    }
    let saved = clean.clone();
    state.store.update(move |d| d.slice_presets = Some(clean)).await?;
    Ok(Json(saved))
}

pub async fn thingiverse(State(state): State<Shared>) -> Json<Value> {
    let token = state.store.read(|d| d.thingiverse_token.clone()).await;
    Json(json!({ "has_token": token.is_some() }))
}

#[derive(Deserialize)]
pub struct AppTokenBody {
    token: String,
}

/// Save the app token after Thingiverse accepts it.
pub async fn set_thingiverse(State(state): State<Shared>, Json(body): Json<AppTokenBody>) -> ApiResult<Json<Value>> {
    let token = body.token.trim().trim_start_matches("Bearer ").trim().to_string();
    if token.is_empty() {
        return Err(AppError::bad_request("paste the app token"));
    }
    crate::sources::thingiverse::check_token(&state.http, &token).await.map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    state.store.update(move |d| d.thingiverse_token = Some(token)).await?;
    Ok(Json(json!({ "has_token": true })))
}

pub async fn remove_thingiverse(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    state.store.update(|d| d.thingiverse_token = None).await?;
    Ok(Json(json!({ "has_token": false })))
}

#[derive(Deserialize)]
pub struct SpoolmanBody {
    url: String,
    #[serde(default)]
    book_usage: bool,
}

pub async fn spoolman(State(state): State<Shared>) -> Json<Value> {
    let s = state.store.read(|d| d.spoolman.clone()).await;
    Json(json!({ "url": s.url, "book_usage": s.book_usage }))
}

/// Save the address (empty turns Spoolman off); manual tray links are kept.
pub async fn set_spoolman(State(state): State<Shared>, Json(body): Json<SpoolmanBody>) -> ApiResult<Json<Value>> {
    let url = body.url.trim().trim_end_matches('/').to_string();
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::bad_request("the Spoolman address starts with http:// or https://"));
    }
    let book = body.book_usage && !url.is_empty();
    state
        .store
        .update(|d| {
            d.spoolman.url = url.clone();
            d.spoolman.book_usage = book;
        })
        .await?;
    crate::spoolman::forget_cache();
    Ok(Json(json!({ "url": url, "book_usage": book })))
}

/// Check an address before saving: how many spools it has.
pub async fn spoolman_test(State(state): State<Shared>, Json(body): Json<SpoolmanBody>) -> ApiResult<Json<Value>> {
    crate::spoolman::forget_cache();
    let s = SpoolmanSettings { url: body.url, ..Default::default() };
    let list = crate::spoolman::spools(&state, &s).await.map_err(|e| AppError::bad_gateway(format!("{e:#}")))?;
    crate::spoolman::forget_cache();
    Ok(Json(json!({ "spools": list.len() })))
}

pub async fn cost(State(state): State<Shared>) -> Json<CostSettings> {
    Json(state.store.read(|d| d.cost.clone()).await)
}

pub async fn set_cost(State(state): State<Shared>, Json(mut body): Json<CostSettings>) -> ApiResult<Json<CostSettings>> {
    for v in [body.price_per_kg, body.per_hour].into_iter().flatten() {
        if !(0.0..=100_000.0).contains(&v) {
            return Err(AppError::bad_request("prices must be between 0 and 100 000"));
        }
    }
    body.currency = body.currency.trim().chars().take(8).collect();
    body.price_per_kg = body.price_per_kg.filter(|v| *v > 0.0);
    body.per_hour = body.per_hour.filter(|v| *v > 0.0);
    let saved = body.clone();
    state.store.update(move |d| d.cost = body).await?;
    Ok(Json(saved))
}

/// Free and total bytes on the disk holding `path`.
fn disk_space(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` a properly sized out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    Some((st.f_bavail as u64 * block, st.f_blocks as u64 * block))
}

pub async fn storage(State(state): State<Shared>) -> Json<Value> {
    let s = state.store.read(|d| d.storage).await;
    let (with_files, removed) = state
        .store
        .read(|d| (d.jobs.iter().filter(|j| !j.files_removed).count(), d.jobs.iter().filter(|j| j.files_removed).count()))
        .await;
    let jobs_dir = state.cfg.jobs_dir();
    let used = tokio::task::spawn_blocking(move || crate::jobs::dir_size(&jobs_dir)).await.unwrap_or(0);
    let (free, total) = disk_space(&state.cfg.data_dir).unwrap_or((0, 0));
    Json(json!({
        "keep_files_days": s.keep_files_days,
        "keep_files_count": s.keep_files_count,
        "used_bytes": used,
        "jobs_with_files": with_files,
        "jobs_cleaned": removed,
        "free_bytes": free,
        "total_bytes": total,
    }))
}

pub async fn set_storage(State(state): State<Shared>, Json(body): Json<StorageSettings>) -> ApiResult<Json<Value>> {
    if body.keep_files_days > 3650 {
        return Err(AppError::bad_request("keep files for at most 3650 days (0 = no limit by age)"));
    }
    if body.keep_files_count > 10_000 {
        return Err(AppError::bad_request("keep files for at most 10 000 jobs (0 = no limit by number)"));
    }
    state.store.update(|d| d.storage = body).await?;
    Ok(storage(State(state)).await)
}

/// Clean up now with the saved setting.
pub async fn clean_storage(State(state): State<Shared>) -> ApiResult<Json<Value>> {
    let keep = state.store.read(|d| d.storage).await;
    let (jobs, bytes) = crate::jobs::clean_files(&state, keep).await?;
    Ok(Json(json!({ "jobs": jobs, "bytes": bytes })))
}
