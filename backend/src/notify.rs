//! Push notifications: a Home Assistant notify service, a Telegram bot, an
//! ntfy topic or a browser (`webpush`). Only the first needs Home Assistant.

use crate::{
    i18n::Lang,
    models::{NotifySettings, NotifyTarget, Printer},
    state::Shared,
};
use anyhow::{Context, anyhow, bail};
use std::path::PathBuf;
use serde_json::{Value, json};

/// Inside the Home Assistant add-on the Supervisor proxies Home Assistant's
/// API with this token.
fn supervisor_token() -> Option<String> {
    std::env::var("SUPERVISOR_TOKEN").ok().filter(|t| !t.is_empty())
}

/// A Home Assistant entity id of `domain` ("camera", "switch", "sensor"):
/// object ids are lower-case letters, digits and underscores. Checked before
/// one goes into an API path or a template, where `../` or quotes would reach
/// other endpoints with the Home Assistant token.
pub fn is_entity_id(id: &str, domain: &str) -> bool {
    id.strip_prefix(domain).and_then(|r| r.strip_prefix('.')).is_some_and(is_object_id)
}

/// A notify service name without `notify.`, e.g. `mobile_app_my_phone`.
pub fn is_object_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Base URL and token of Home Assistant's REST API.
pub(crate) fn endpoint(s: &NotifySettings) -> anyhow::Result<(String, String)> {
    if !s.ha_url.trim().is_empty() {
        if s.token.trim().is_empty() {
            bail!("add a long-lived access token from your Home Assistant profile");
        }
        return Ok((format!("{}/api", s.ha_url.trim().trim_end_matches('/')), s.token.trim().to_string()));
    }
    match supervisor_token() {
        Some(t) => Ok(("http://supervisor/core/api".into(), t)),
        None => bail!("enter your Home Assistant address and a long-lived access token"),
    }
}

/// Where a tapped notification goes: the configured link, else the add-on's
/// panel in the Home Assistant app.
async fn open_url(state: &Shared, s: &NotifySettings) -> Option<String> {
    if !s.open_url.trim().is_empty() {
        return Some(s.open_url.trim().to_string());
    }
    let token = supervisor_token()?;
    let info: Value = state.http.get("http://supervisor/addons/self/info").bearer_auth(token).send().await.ok()?.json().await.ok()?;
    let slug = info.pointer("/data/slug")?.as_str()?;
    Some(format!("/hassio/ingress/{slug}"))
}

/// A picture to attach: JPEG from a camera, PNG of a plate, or the model
/// site's own picture (JPEG, PNG or WebP).
#[derive(Clone)]
pub struct Picture {
    pub bytes: std::sync::Arc<Vec<u8>>,
    /// `image/jpeg`, `image/png` or `image/webp`.
    pub mime: &'static str,
}

impl Picture {
    pub(crate) fn mime(&self) -> &'static str {
        self.mime
    }
    fn file_name(&self, i: usize) -> String {
        if i == 0 { format!("picture.{}", self.ext()) } else { format!("picture-{i}.{}", self.ext()) }
    }
    fn ext(&self) -> &'static str {
        match self.mime {
            "image/png" => "png",
            "image/webp" => "webp",
            _ => "jpg",
        }
    }
}

/// One notification.
pub struct Message<'a> {
    pub title: &'a str,
    pub message: &'a str,
    /// Pictures to attach, most telling first (Telegram sends all, ntfy the first).
    pub photos: &'a [Picture],
    /// A picture Home Assistant can fetch itself, e.g. `/api/camera_proxy/camera.x`.
    pub ha_image: Option<String>,
    /// The job it is about; a browser notification opens it.
    pub job: Option<&'a str>,
}

/// Send one notification to one target now; errors come back to the caller.
pub async fn send_now(state: &Shared, s: &NotifySettings, target: &NotifyTarget, title: &str, message: &str) -> anyhow::Result<()> {
    deliver(state, s, target, &Message { title, message, photos: &[], ha_image: None, job: None }).await
}

async fn deliver(state: &Shared, s: &NotifySettings, t: &NotifyTarget, m: &Message<'_>) -> anyhow::Result<()> {
    match t.channel.as_str() {
        "telegram" => telegram(state, s, t, m).await,
        "ntfy" => ntfy(state, s, t, m).await,
        "webpush" => crate::webpush::send(state, t, m.title, m.message, m.job, m.photos.first()).await,
        _ => home_assistant(state, s, t, m).await,
    }
}

async fn home_assistant(state: &Shared, s: &NotifySettings, t: &NotifyTarget, m: &Message<'_>) -> anyhow::Result<()> {
    let (base, token) = endpoint(s)?;
    let service = t.service.trim().trim_start_matches("notify.");
    if service.is_empty() {
        bail!("choose a notify service");
    }
    if !is_object_id(service) {
        bail!("{service:?} isn't a notify service name");
    }
    let mut body = json!({ "title": m.title, "message": m.message, "data": { "group": "airslice" } });
    if let Some(url) = open_url(state, s).await {
        body["data"]["url"] = json!(url);
        body["data"]["clickAction"] = json!(url);
    }
    if let Some(image) = &m.ha_image {
        body["data"]["image"] = json!(image);
    }
    let resp = state
        .http
        .post(format!("{base}/services/notify/{service}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("couldn't reach Home Assistant")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Home Assistant answered {status}: {}", text.chars().take(200).collect::<String>());
    }
    Ok(())
}

/// Telegram's API; tests point it elsewhere.
fn telegram_api() -> String {
    std::env::var("AIRSLICE_TELEGRAM_API").unwrap_or_else(|_| "https://api.telegram.org".into())
}

async fn telegram(state: &Shared, s: &NotifySettings, t: &NotifyTarget, m: &Message<'_>) -> anyhow::Result<()> {
    let (token, chat) = (t.telegram_token.trim(), t.telegram_chat.trim());
    if token.is_empty() || chat.is_empty() {
        bail!("enter the bot token and the chat id");
    }
    let mut text = format!("{}\n{}", m.title, m.message);
    if !s.open_url.trim().is_empty() {
        text.push_str(&format!("\n{}", s.open_url.trim()));
    }
    let base = format!("{}/bot{token}", telegram_api());
    let req = match m.photos {
        [] => state.http.post(format!("{base}/sendMessage")).json(&json!({ "chat_id": chat, "text": text })),
        [img] => {
            let form = reqwest::multipart::Form::new()
                .text("chat_id", chat.to_string())
                .text("caption", text)
                .part("photo", reqwest::multipart::Part::bytes(img.bytes.to_vec()).file_name(img.file_name(0)).mime_str(img.mime())?);
            state.http.post(format!("{base}/sendPhoto")).multipart(form)
        }
        // An album; the caption on the first picture shows under it.
        many => {
            let media: Vec<Value> = (0..many.len())
                .map(|i| {
                    let mut p = json!({ "type": "photo", "media": format!("attach://p{i}") });
                    if i == 0 {
                        p["caption"] = json!(text);
                    }
                    p
                })
                .collect();
            let mut form = reqwest::multipart::Form::new().text("chat_id", chat.to_string()).text("media", media_json(&media));
            for (i, img) in many.iter().enumerate() {
                form = form.part(format!("p{i}"), reqwest::multipart::Part::bytes(img.bytes.to_vec()).file_name(img.file_name(i)).mime_str(img.mime())?);
            }
            state.http.post(format!("{base}/sendMediaGroup")).multipart(form)
        }
    };
    let resp = req.send().await.context("couldn't reach Telegram")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or_default();
        let why = v.get("description").and_then(|d| d.as_str()).unwrap_or("");
        bail!("Telegram answered {status}{}", if why.is_empty() { String::new() } else { format!(": {why}") });
    }
    Ok(())
}

fn media_json(media: &[Value]) -> String {
    Value::Array(media.to_vec()).to_string()
}

/// Header values must be plain text; ntfy reads UTF-8 in RFC 2047 form.
fn header_text(v: &str) -> String {
    if v.is_ascii() {
        v.replace(['\r', '\n'], " ")
    } else {
        use base64::Engine;
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(v.replace(['\r', '\n'], " ")))
    }
}

async fn ntfy(state: &Shared, s: &NotifySettings, t: &NotifyTarget, m: &Message<'_>) -> anyhow::Result<()> {
    let url = t.ntfy_url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("enter the topic's address, e.g. https://ntfy.sh/my-printer");
    }
    let mut req = match m.photos.first() {
        // A picture is the body; the text goes in a header.
        Some(img) => state
            .http
            .put(url)
            .header("Filename", img.file_name(0))
            .header("Message", header_text(m.message))
            .body(img.bytes.to_vec()),
        None => state.http.post(url).body(m.message.to_string()),
    };
    req = req.header("Title", header_text(m.title)).header("Tags", "printer");
    if !s.open_url.trim().is_empty() {
        req = req.header("Click", s.open_url.trim());
    }
    if !t.ntfy_token.trim().is_empty() {
        req = req.bearer_auth(t.ntfy_token.trim());
    }
    let resp = req.send().await.context("couldn't reach ntfy")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("ntfy answered {status}: {}", text.chars().take(200).collect::<String>());
    }
    Ok(())
}

/// What to picture in a notification.
#[derive(Default)]
pub struct Pictures {
    /// Take a camera picture of this printer, when it has a camera.
    pub camera: Option<Printer>,
    /// A plate render: (job id, file in the job folder, plate).
    pub plate: Option<(String, PathBuf, u32)>,
    /// Attach the plate render even when there is a camera picture.
    pub both: bool,
    /// Pictures from the model site, tried in turn when there is no plate
    /// render (a mesh sliced on the server has none).
    pub site: Vec<String>,
}

/// Home Assistant's media folder, mapped into the add-on (`map: media:rw`).
const HA_MEDIA: &str = "/media/airslice";

/// The words of a notification in a given language: (title, message).
pub type Text = Box<dyn Fn(Lang) -> (String, String) + Send + Sync>;

/// Home Assistant's own language, asked once ("" when it can't be reached).
async fn ha_language(state: &Shared, s: &NotifySettings) -> Lang {
    static CODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(c) = CODE.get() {
        return Lang::from_code(c);
    }
    let mut code = String::new();
    if let Ok((base, token)) = endpoint(s)
        && let Ok(r) = state.http.get(format!("{base}/config")).bearer_auth(token).send().await
        && let Ok(v) = r.json::<Value>().await
    {
        code = v.get("language").and_then(|l| l.as_str()).unwrap_or_default().to_string();
        let _ = CODE.set(code.clone());
    }
    Lang::from_code(&code)
}

/// A target's language: its own choice, else Home Assistant's.
pub async fn language(state: &Shared, s: &NotifySettings, t: &NotifyTarget) -> Lang {
    match t.language.as_str() {
        "" => ha_language(state, s).await,
        code => Lang::from_code(code),
    }
}

/// Send in the background to every notification that is on and wants this
/// kind of event (`wanted`), each in its own language, with pictures; failures
/// are only logged. The camera picture is taken once and shared.
pub fn send_with_pictures(state: &Shared, wanted: impl Fn(&NotifyTarget) -> bool + Send + 'static, text: Text, pics: Pictures) {
    let state = state.clone();
    tokio::spawn(async move {
        let s = state.store.read(|d| d.notify.clone()).await;
        if !s.enabled {
            return;
        }
        let targets: Vec<&NotifyTarget> = s.targets.iter().filter(|t| t.enabled && wanted(t)).collect();
        if targets.is_empty() || !targets.iter().any(|t| t.photos) {
            for t in targets {
                let (title, message) = text(language(&state, &s, t).await);
                let m = Message { title: &title, message: &message, photos: &[], ha_image: None, job: pics.plate.as_ref().map(|p| p.0.as_str()) };
                if let Err(e) = deliver(&state, &s, t, &m).await {
                    tracing::warn!("notification {:?} failed: {e:#}", t.name);
                }
            }
            return;
        }
        let camera = pics.camera.filter(|p| !p.camera.is_empty());
        let mut photos = Vec::new();
        if let Some(p) = &camera {
            match crate::camera::snapshot(&state, p).await {
                Ok(img) => photos.push(Picture { bytes: img, mime: "image/jpeg" }),
                Err(e) => tracing::warn!("no camera picture for the notification: {e:#}"),
            }
        }
        if let Some((_, path, plate)) = &pics.plate
            && (photos.is_empty() || pics.both)
        {
            let (path, plate) = (path.clone(), *plate);
            if let Ok(Ok(png)) = tokio::task::spawn_blocking(move || crate::threemf::thumbnail(&path, plate)).await {
                photos.push(Picture { bytes: std::sync::Arc::new(png), mime: "image/png" });
            }
        }
        if photos.is_empty() {
            for url in &pics.site {
                match crate::api::jobs::site_picture(url).await {
                    Ok(p) => {
                        photos.push(p);
                        break;
                    }
                    Err(e) => tracing::warn!("no model picture for the notification ({url}): {e:#}"),
                }
            }
        }
        // Home Assistant fetches the picture itself: its own camera, else a
        // copy in its media folder (only inside the add-on).
        let ha_image = match camera.as_ref().filter(|p| p.camera == "ha").and_then(|p| p.camera_entity.clone()).filter(|e| is_entity_id(e, "camera")) {
            Some(e) => Some(format!("/api/camera_proxy/{e}")),
            None if targets.iter().any(|t| t.photos && t.channel == "ha") => match photos.first() {
                Some(img) => to_ha_media(img, pics.plate.as_ref().map(|p| p.0.as_str()).unwrap_or("print")).await,
                None => None,
            },
            None => None,
        };
        for t in targets {
            let (title, message) = text(language(&state, &s, t).await);
            let m = Message {
                title: &title,
                message: &message,
                photos: if t.photos && t.channel != "ha" { &photos } else { &[] },
                ha_image: ha_image.clone().filter(|_| t.photos && t.channel == "ha"),
                job: pics.plate.as_ref().map(|p| p.0.as_str()),
            };
            if let Err(e) = deliver(&state, &s, t, &m).await {
                tracing::warn!("notification {:?} failed: {e:#}", t.name);
            }
        }
    });
}

/// Put a picture in Home Assistant's media folder and return the path the
/// companion app loads it from; pictures older than a week are removed.
async fn to_ha_media(img: &Picture, name: &str) -> Option<String> {
    let dir = std::path::Path::new(HA_MEDIA);
    if !dir.parent().is_some_and(|p| p.is_dir()) {
        return None;
    }
    tokio::fs::create_dir_all(dir).await.ok()?;
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(e)) = entries.next_entry().await {
            let old = e.metadata().await.ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 7 * 86_400);
            if old {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    let file = format!("{name}-{}.{}", crate::models::now(), img.ext());
    tokio::fs::write(dir.join(&file), img.bytes.as_slice()).await.ok()?;
    Some(format!("/media/local/airslice/{file}"))
}

/// The notify services Home Assistant offers, e.g. `mobile_app_my_phone`.
pub async fn services(state: &Shared, s: &NotifySettings) -> anyhow::Result<Vec<String>> {
    let (base, token) = endpoint(s)?;
    let all: Value = state
        .http
        .get(format!("{base}/services"))
        .bearer_auth(token)
        .send()
        .await
        .context("couldn't reach Home Assistant")?
        .error_for_status()
        .map_err(|e| anyhow!("Home Assistant refused: {e}"))?
        .json()
        .await?;
    let mut out: Vec<String> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d.get("domain").and_then(|x| x.as_str()) == Some("notify"))
        .filter_map(|d| d.get("services")?.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()))
        .flatten()
        .filter(|n| n != "persistent_notification" && n != "send_message")
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_cant_leave_their_path() {
        assert!(is_entity_id("camera.pandan_camera", "camera"));
        assert!(is_entity_id("switch.pandan_wall_plug", "switch"));
        assert!(!is_entity_id("camera.x/../../states", "camera"));
        assert!(!is_entity_id("camera.%2e%2e", "camera"));
        assert!(!is_entity_id("switch.a')", "switch"));
        assert!(!is_entity_id("switch.pandan", "camera"));
        assert!(!is_entity_id("camera.", "camera"));
        assert!(is_object_id("mobile_app_anders_iphone"));
        assert!(!is_object_id("../homeassistant/restart"));
    }
}
