use crate::{
    auth::{self, Principal},
    error::{ApiResult, AppError},
    models::{ApiKey, Session, now},
    state::Shared,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct PasswordBody {
    password: String,
}

pub async fn status(State(state): State<Shared>, req: axum::extract::Request) -> Json<Value> {
    let who = auth::principal(&state, req.headers(), req.extensions()).await;
    let ingress = matches!(who, Some(Principal::Ingress { .. }));
    Json(json!({
        // Through Home Assistant the user is already signed in.
        "setup_required": !ingress && auth::setup_required(&state).await,
        // In the add-on the first password is set in the Home Assistant panel,
        // never on the open port (whoever came first would own AirSlice).
        "setup_blocked": state.cfg.ingress_peer.is_some(),
        "authenticated": who.is_some(),
        "ingress": ingress,
        "user": match &who { Some(Principal::Ingress { user }) => Some(user.clone()), _ => None },
        "admin_password_set": !auth::setup_required(&state).await,
        "password_from_env": state.cfg.admin_password.is_some(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn start_session(state: &Shared, headers: &HeaderMap) -> ApiResult<Response> {
    let token = auth::random_token();
    let t = now();
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect();
    state
        .store
        .update(|d| {
            d.sessions.retain(|s| t - s.created_at < 365 * 86400);
            d.sessions.push(Session { hash: auth::sha256_hex(&token), created_at: t, last_seen: t, user_agent: ua });
            // Keep the list bounded.
            if d.sessions.len() > 50 {
                d.sessions.sort_by_key(|s| std::cmp::Reverse(s.last_seen));
                d.sessions.truncate(50);
            }
        })
        .await?;
    Ok(([(header::SET_COOKIE, auth::session_cookie(state, headers, &token))], Json(json!({"ok": true}))).into_response())
}

/// A private, link-local or CGNAT (Tailscale and the like) address.
fn is_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])),
        std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

pub async fn setup(
    State(state): State<Shared>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PasswordBody>,
) -> ApiResult<Response> {
    if !auth::setup_required(&state).await {
        return Err(AppError::conflict("already set up"));
    }
    if state.cfg.ingress_peer.is_some() {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "set the first password in the AirSlice panel in Home Assistant (Settings, Access) or in the add-on's options",
        ));
    }
    // Until a password is set, anyone who reaches the page could set one:
    // only from the local network (or a reverse proxy on it).
    if !peer.ip().to_canonical().is_loopback() && !is_local(peer.ip().to_canonical()) {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "set the first password from your own network, or start AirSlice with ADMIN_PASSWORD",
        ));
    }
    if body.password.chars().count() < 8 {
        return Err(AppError::bad_request("use at least 8 characters"));
    }
    let hash = auth::hash_password(&body.password)?;
    state.store.update(|d| d.admin_password_hash = Some(hash)).await?;
    start_session(&state, &headers).await
}

/// Wrong passwords: more than `MAX_FAILURES` within `FAILURE_WINDOW` and
/// logins are refused for `LOCKOUT` seconds. Counted for the whole server
/// (addresses behind a proxy say little); Home Assistant's panel doesn't log in.
const MAX_FAILURES: usize = 10;
const FAILURE_WINDOW: i64 = 600;
const LOCKOUT: i64 = 60;

static FAILURES: std::sync::Mutex<Vec<i64>> = std::sync::Mutex::new(Vec::new());
/// One password check at a time, so parallel guesses don't add up.
static CHECKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn locked_for(now: i64) -> Option<i64> {
    let mut f = FAILURES.lock().unwrap_or_else(|e| e.into_inner());
    f.retain(|t| now - t < FAILURE_WINDOW);
    let last = *f.last()?;
    (f.len() >= MAX_FAILURES && now - last < LOCKOUT).then(|| LOCKOUT - (now - last))
}

pub async fn login(State(state): State<Shared>, headers: HeaderMap, Json(body): Json<PasswordBody>) -> ApiResult<Response> {
    if auth::setup_required(&state).await {
        return Err(AppError::conflict("set a password first"));
    }
    let _one = CHECKING.lock().await;
    if let Some(secs) = locked_for(crate::models::now()) {
        return Err(AppError::new(StatusCode::TOO_MANY_REQUESTS, format!("too many wrong passwords; try again in {secs} s")));
    }
    if !auth::check_admin_password(&state, &body.password).await {
        FAILURES.lock().unwrap_or_else(|e| e.into_inner()).push(crate::models::now());
        // Slow down guessing (the lock above makes this apply to everyone).
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        return Err(AppError::unauthorized("wrong password"));
    }
    start_session(&state, &headers).await
}

pub async fn logout(State(state): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::cookie_value(&headers, auth::COOKIE) {
        let hash = auth::sha256_hex(&token);
        state.store.update(|d| d.sessions.retain(|s| s.hash != hash)).await?;
    }
    Ok(([(header::SET_COOKIE, auth::clear_cookie(&state))], Json(json!({"ok": true}))).into_response())
}

#[derive(Deserialize)]
pub struct ChangePassword {
    #[serde(default)]
    current: String,
    new: String,
}

pub async fn change_password(
    State(state): State<Shared>,
    Extension(who): Extension<Principal>,
    Json(body): Json<ChangePassword>,
) -> ApiResult<Json<Value>> {
    if !who.is_interactive() {
        return Err(AppError::new(StatusCode::FORBIDDEN, "API keys can't change the password"));
    }
    if state.cfg.admin_password.is_some() {
        return Err(AppError::conflict("the password is set by ADMIN_PASSWORD in the container environment"));
    }
    // From Home Assistant, the first password (for direct access) can be set
    // without a current one.
    let first_from_ha = matches!(who, Principal::Ingress { .. }) && auth::setup_required(&state).await;
    if !first_from_ha && !auth::check_admin_password(&state, &body.current).await {
        return Err(AppError::unauthorized("current password is wrong"));
    }
    if body.new.chars().count() < 8 {
        return Err(AppError::bad_request("use at least 8 characters"));
    }
    let hash = auth::hash_password(&body.new)?;
    state.store.update(|d| d.admin_password_hash = Some(hash)).await?;
    Ok(Json(json!({"ok": true})))
}

/// Sign out every browser (API keys are unaffected).
pub async fn revoke_sessions(State(state): State<Shared>) -> ApiResult<Response> {
    state.store.update(|d| d.sessions.clear()).await?;
    Ok(([(header::SET_COOKIE, auth::clear_cookie(&state))], Json(json!({"ok": true}))).into_response())
}

// ------------------------------------------------------------------ API keys

pub async fn list_keys(State(state): State<Shared>) -> Json<Vec<ApiKey>> {
    let keys = state
        .store
        .read(|d| d.api_keys.iter().cloned().map(|mut k| {
            k.hash.clear();
            k
        }).collect())
        .await;
    Json(keys)
}

#[derive(Deserialize)]
pub struct NewKey {
    name: String,
}

pub async fn create_key(
    State(state): State<Shared>,
    Extension(who): Extension<Principal>,
    Json(body): Json<NewKey>,
) -> ApiResult<Json<Value>> {
    if !who.is_interactive() {
        return Err(AppError::new(StatusCode::FORBIDDEN, "create keys from the web UI"));
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("give the key a name, e.g. \"Home Assistant\""));
    }
    let key = format!("{}{}", auth::API_KEY_PREFIX, auth::random_token());
    let record = ApiKey {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        name: name.chars().take(60).collect(),
        prefix: key.chars().take(10).collect(),
        hash: auth::sha256_hex(&key),
        created_at: now(),
        last_used_at: None,
    };
    let mut shown = record.clone();
    shown.hash.clear();
    state.store.update(|d| d.api_keys.push(record)).await?;
    // The full key is only ever returned here.
    Ok(Json(json!({ "key": key, "record": shown })))
}

pub async fn delete_key(State(state): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let removed = state
        .store
        .update(|d| {
            let before = d.api_keys.len();
            d.api_keys.retain(|k| k.id != id);
            before != d.api_keys.len()
        })
        .await?;
    if !removed {
        return Err(AppError::not_found("key not found"));
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_after_many_wrong_passwords() {
        let now = 1_000_000;
        FAILURES.lock().unwrap().clear();
        for i in 0..MAX_FAILURES as i64 - 1 {
            FAILURES.lock().unwrap().push(now - 100 + i);
        }
        assert_eq!(locked_for(now), None, "one short of the limit");
        FAILURES.lock().unwrap().push(now - 5);
        assert_eq!(locked_for(now), Some(LOCKOUT - 5));
        assert_eq!(locked_for(now + LOCKOUT), None, "open again after the lockout");
        assert_eq!(locked_for(now + FAILURE_WINDOW + 1), None);
        FAILURES.lock().unwrap().clear();
    }

    #[test]
    fn local_addresses() {
        for ip in ["192.168.1.5", "10.0.0.2", "172.20.1.1", "100.101.2.3", "169.254.1.1", "fd12::1", "fe80::1"] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.200.1.1", "2001:db8::1", "172.32.0.1"] {
            assert!(!is_local(ip.parse().unwrap()), "{ip}");
        }
    }
}
