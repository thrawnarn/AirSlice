//! Admin password, browser sessions and API keys.
//!
//! * The web UI signs in with the admin password and gets a long-lived,
//!   HttpOnly session cookie (so the home-screen app stays signed in).
//! * Integrations (Home Assistant, iOS Shortcuts) use API keys:
//!   `Authorization: Bearer as_…` or `X-API-Key: as_…`.

use crate::{error::AppError, models::now, state::Shared};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, header},
    middleware::Next,
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "as_session";
pub const API_KEY_PREFIX: &str = "as_";
const SESSION_DAYS: i64 = 365;

#[derive(Debug, Clone)]
pub enum Principal {
    Session,
    ApiKey { id: String, name: String },
    /// Signed in to Home Assistant, reaching us through add-on ingress.
    Ingress { user: String },
}

impl Principal {
    /// A person using the UI (not an integration).
    pub fn is_interactive(&self) -> bool {
        matches!(self, Principal::Session | Principal::Ingress { .. })
    }
}

/// Set by `mark_ingress` for requests proxied by Home Assistant ingress.
#[derive(Debug, Clone)]
pub struct IngressUser(pub String);

/// Outermost middleware: recognise Home Assistant ingress requests. Only the
/// Supervisor's address is trusted, and only when configured, so the header
/// can't be used to bypass sign-in from elsewhere.
pub async fn mark_ingress(State(state): State<Shared>, mut req: Request, next: Next) -> Response {
    req.extensions_mut().remove::<IngressUser>();
    if let Some(trusted) = state.cfg.ingress_peer {
        let peer = req
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip());
        if peer == Some(trusted) && req.headers().contains_key("x-ingress-path") {
            let user = ["x-remote-user-display-name", "x-remote-user-name"]
                .iter()
                .find_map(|h| req.headers().get(*h).and_then(|v| v.to_str().ok()).filter(|v| !v.is_empty()))
                .unwrap_or("Home Assistant")
                .to_string();
            req.extensions_mut().insert(IngressUser(user));
        }
    }
    next.run(req).await
}

pub fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random generator");
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn hash_password(pw: &str) -> anyhow::Result<String> {
    let hash = Argon2::default().hash_password(pw.as_bytes()).map_err(|e| anyhow::anyhow!("hashing failed: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(pw: &str, stored: &str) -> bool {
    Argon2::default().verify_password(pw.as_bytes(), stored).is_ok()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Check the admin password against env or the stored hash.
pub async fn check_admin_password(state: &Shared, pw: &str) -> bool {
    if let Some(env_pw) = &state.cfg.admin_password {
        return constant_time_eq(env_pw.as_bytes(), pw.as_bytes());
    }
    match state.store.read(|d| d.admin_password_hash.clone()).await {
        Some(hash) => {
            let pw = pw.to_string();
            tokio::task::spawn_blocking(move || verify_password(&pw, &hash)).await.unwrap_or(false)
        }
        None => false,
    }
}

pub async fn setup_required(state: &Shared) -> bool {
    state.cfg.admin_password.is_none() && state.store.read(|d| d.admin_password_hash.is_none()).await
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn api_key_from(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(v.trim().to_string());
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|v| v.trim().to_string())
}

/// Whether the browser reached us over HTTPS (directly or via a proxy).
pub fn is_https(state: &Shared, headers: &HeaderMap) -> bool {
    if let Some(forced) = state.cfg.cookie_secure {
        return forced;
    }
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
        || headers
            .get("forwarded")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_lowercase().contains("proto=https"))
            .unwrap_or(false)
}

pub fn session_cookie(state: &Shared, headers: &HeaderMap, token: &str) -> String {
    let path = if state.cfg.base_path.is_empty() { "/".to_string() } else { state.cfg.base_path.clone() };
    let secure = if is_https(state, headers) { "; Secure" } else { "" };
    format!("{COOKIE}={token}; Path={path}; HttpOnly; SameSite=Lax; Max-Age={}{secure}", SESSION_DAYS * 86400)
}

pub fn clear_cookie(state: &Shared) -> String {
    let path = if state.cfg.base_path.is_empty() { "/".to_string() } else { state.cfg.base_path.clone() };
    format!("{COOKIE}=; Path={path}; HttpOnly; SameSite=Lax; Max-Age=0")
}

pub async fn principal(state: &Shared, headers: &HeaderMap, ext: &axum::http::Extensions) -> Option<Principal> {
    if let Some(IngressUser(user)) = ext.get::<IngressUser>() {
        return Some(Principal::Ingress { user: user.clone() });
    }
    let t = now();
    if let Some(key) = api_key_from(headers).filter(|k| k.starts_with(API_KEY_PREFIX)) {
        let hash = sha256_hex(&key);
        let found = state.store.read(|d| d.api_keys.iter().find(|k| k.hash == hash).cloned()).await?;
        if found.last_used_at.map(|l| t - l > 300).unwrap_or(true) {
            let _ = state
                .store
                .update(|d| {
                    if let Some(k) = d.api_keys.iter_mut().find(|k| k.id == found.id) {
                        k.last_used_at = Some(t);
                    }
                })
                .await;
        }
        return Some(Principal::ApiKey { id: found.id, name: found.name });
    }
    let token = cookie_value(headers, COOKIE)?;
    let hash = sha256_hex(&token);
    let session = state.store.read(|d| d.sessions.iter().find(|s| s.hash == hash).cloned()).await?;
    if t - session.created_at > SESSION_DAYS * 86400 {
        return None;
    }
    if t - session.last_seen > 3600 {
        let _ = state
            .store
            .update(|d| {
                if let Some(s) = d.sessions.iter_mut().find(|s| s.hash == hash) {
                    s.last_seen = t;
                }
            })
            .await;
    }
    Some(Principal::Session)
}

/// Middleware for everything under /api except the auth endpoints.
pub async fn require_auth(State(state): State<Shared>, mut req: Request, next: Next) -> Result<Response, AppError> {
    match principal(&state, req.headers(), req.extensions()).await {
        Some(p) => {
            if let Principal::ApiKey { id, name } = &p {
                tracing::debug!(key_id = %id, "API key {name:?}: {} {}", req.method(), req.uri().path());
            }
            req.extensions_mut().insert(p);
            Ok(next.run(req).await)
        }
        None => Err(AppError::unauthorized("sign in or send an API key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn tokens_are_random() {
        let a = random_token();
        assert_ne!(a, random_token());
        assert!(a.len() >= 43);
    }

    #[test]
    fn reads_cookie_and_keys() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; as_session=tok; b=2"));
        assert_eq!(cookie_value(&h, COOKIE).as_deref(), Some("tok"));
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer as_abc"));
        assert_eq!(api_key_from(&h).as_deref(), Some("as_abc"));
        h.insert("x-api-key", HeaderValue::from_static("as_xyz"));
        assert_eq!(api_key_from(&h).as_deref(), Some("as_xyz"));
    }

    #[test]
    fn ct_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
