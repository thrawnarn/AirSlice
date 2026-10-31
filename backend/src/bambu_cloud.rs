//! Signing in to a Bambu account — only to download from MakerWorld.

use crate::sources::makerworld::api_base;
use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginStep {
    /// Signed in; token returned.
    Done { #[serde(skip)] token: String },
    /// A code was e-mailed; call `verify_email_code`.
    EmailCode,
    /// Account uses an authenticator app; not supported here.
    Totp,
}

async fn post_login(client: &reqwest::Client, china: bool, body: Value) -> anyhow::Result<(reqwest::StatusCode, Value)> {
    let resp = client
        .post(format!("{}/v1/user-service/user/login", api_base(china)))
        .json(&body)
        .send()
        .await
        .context("could not reach Bambu's sign-in service")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let value = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
    if status.as_u16() == 418 {
        bail!("Bambu is asking for a CAPTCHA from this server. Try again later, or paste a token instead.");
    }
    Ok((status, value))
}

fn token_of(v: &Value) -> Option<String> {
    v.get("accessToken").or_else(|| v.get("token")).and_then(|t| t.as_str()).filter(|t| !t.is_empty()).map(str::to_string)
}

fn message_of(v: &Value) -> String {
    v.get("message").or_else(|| v.get("error")).and_then(|m| m.as_str()).unwrap_or("sign-in failed").to_string()
}

pub async fn login(client: &reqwest::Client, china: bool, email: &str, password: &str) -> anyhow::Result<LoginStep> {
    let (status, v) = post_login(client, china, json!({"account": email, "password": password})).await?;
    if status.is_success() {
        if let Some(token) = token_of(&v) {
            return Ok(LoginStep::Done { token });
        }
        match v.get("loginType").and_then(|t| t.as_str()) {
            Some("verifyCode") => return Ok(LoginStep::EmailCode),
            Some("tfa") => return Ok(LoginStep::Totp),
            _ if v.get("tfaKey").is_some() => return Ok(LoginStep::Totp),
            _ => {}
        }
    }
    bail!("{}", message_of(&v))
}

pub async fn verify_email_code(client: &reqwest::Client, china: bool, email: &str, code: &str) -> anyhow::Result<String> {
    let (status, v) = post_login(client, china, json!({"account": email, "code": code.trim()})).await?;
    match token_of(&v) {
        Some(t) if status.is_success() => Ok(t),
        _ => bail!("{}", message_of(&v)),
    }
}

/// Cheap check that a token is still accepted.
pub async fn check_token(client: &reqwest::Client, china: bool, token: &str) -> anyhow::Result<bool> {
    let resp = client
        .get(format!("{}/v1/design-user-service/my/profile", api_base(china)))
        .bearer_auth(token)
        .send()
        .await
        .context("could not reach Bambu")?;
    Ok(match resp.status().as_u16() {
        200..=299 => true,
        401 => false,
        s => bail!("Bambu returned HTTP {s}"),
    })
}
