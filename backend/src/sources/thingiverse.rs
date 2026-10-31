//! Thingiverse (thingiverse.com) via its REST API. The API and the file
//! downloads need an app token from thingiverse.com/developers; without one
//! Thingiverse answers 403 to servers.

use super::{ParsedLink, SLICEABLE, extension};
use crate::models::{ModelFile, ResolvedModel, Source};
use anyhow::{Context, anyhow, bail};
use serde_json::Value;
use std::path::Path;

/// Thingiverse's API; tests point it elsewhere.
fn api() -> String {
    std::env::var("AIRSLICE_THINGIVERSE_API").unwrap_or_else(|_| "https://api.thingiverse.com".into())
}

async fn get(client: &reqwest::Client, token: &str, path: &str) -> anyhow::Result<Value> {
    let resp = client.get(format!("{}{path}", api())).bearer_auth(token).send().await.map_err(|e| {
        tracing::warn!("Thingiverse request failed: {e:#}");
        anyhow!("Couldn't reach Thingiverse. Check that the AirSlice server has internet access.")
    })?;
    match resp.status().as_u16() {
        200..=299 => resp.json().await.context("Thingiverse sent something unexpected"),
        401 | 403 => bail!("Thingiverse didn't accept the token. Check it under Settings → Model sites."),
        404 => bail!("Model not found on Thingiverse (it may be private or removed)."),
        429 => bail!("Thingiverse is limiting requests right now. Try again in a minute."),
        s => bail!("Thingiverse answered HTTP {s}"),
    }
}

/// Is the token accepted? Asks for a well-known thing (3DBenchy).
/// Errors other than a refused token count as accepted, so a network hiccup
/// doesn't block saving it.
pub async fn check_token(client: &reqwest::Client, token: &str) -> anyhow::Result<()> {
    match get(client, token, "/things/763622").await {
        Err(e) if e.to_string().contains("didn't accept the token") => Err(e),
        _ => Ok(()),
    }
}

pub async fn resolve(client: &reqwest::Client, link: &ParsedLink, token: &str) -> anyhow::Result<ResolvedModel> {
    let thing = get(client, token, &format!("/things/{}", link.model_id)).await?;
    let files = get(client, token, &format!("/things/{}/files", link.model_id)).await?;
    Ok(to_model(link, &thing, &files))
}

fn to_model(link: &ParsedLink, thing: &Value, files: &Value) -> ResolvedModel {
    let mut out = Vec::new();
    let mut skipped = 0;
    for f in files.as_array().into_iter().flatten() {
        let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let id = match f.get("id") {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        let ext = extension(&name);
        if !SLICEABLE.contains(&ext.as_str()) {
            skipped += 1;
            continue;
        }
        out.push(ModelFile {
            id,
            name,
            kind: if ext == "stp" { "step".into() } else { ext },
            size_bytes: f.get("size").and_then(|s| s.as_u64()),
            thumbnail: f.get("thumbnail").and_then(|t| t.as_str()).filter(|t| t.starts_with("http") && !t.contains("default")).map(str::to_string),
            printer: None,
            compatible: vec![],
            plates: None,
            need_ams: None,
            filaments: vec![],
            download_type: None,
            folder: None,
            prediction_secs: None,
            weight_g: None,
        });
    }
    out.sort_by_key(|f| (f.kind != "3mf", f.name.to_lowercase()));

    let mut notes = Vec::new();
    if out.is_empty() {
        notes.push("This model has no files that can be sliced (STL, 3MF, OBJ, STEP).".into());
    } else if skipped > 0 {
        notes.push(format!("{skipped} file(s) hidden because they can't be sliced (e.g. ZIP, SCAD or images)."));
    }
    let cover = thing
        .pointer("/default_image/sizes")
        .and_then(|s| s.as_array())
        .and_then(|sizes| {
            sizes
                .iter()
                .find(|s| s.get("type").and_then(|t| t.as_str()) == Some("display") && s.get("size").and_then(|t| t.as_str()) == Some("large"))
                .or_else(|| sizes.first())
        })
        .and_then(|s| s.get("url"))
        .or_else(|| thing.get("thumbnail"))
        .and_then(|u| u.as_str())
        .filter(|u| u.starts_with("http"))
        .map(str::to_string);
    ResolvedModel {
        source: Source::Thingiverse,
        model_id: link.model_id.clone(),
        url: link.url.clone(),
        title: thing.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| format!("Thingiverse thing {}", link.model_id)),
        author: thing.pointer("/creator/name").and_then(|n| n.as_str()).map(str::to_string),
        cover,
        files: out,
        preselected_file_id: None,
        notes,
        internal_id: None,
        details: None,
    }
}

/// Download a file: the API redirects to the file host (the token isn't
/// passed on to it). Returns bytes written.
pub async fn download(client: &reqwest::Client, token: &str, file_id: &str, dest: &Path) -> anyhow::Result<u64> {
    let url = format!("{}/files/{file_id}/download", api());
    let resp = client.get(&url).bearer_auth(token).send().await.map_err(|e| {
        tracing::warn!("Thingiverse download failed: {e:#}");
        anyhow!("Download failed — couldn't reach Thingiverse.")
    })?;
    match resp.status().as_u16() {
        200..=299 => {}
        401 | 403 => bail!("Thingiverse didn't accept the token for this download. Check it under Settings → Model sites."),
        s => bail!("Thingiverse download failed: HTTP {s}"),
    }
    // Some answers are JSON with the file's address instead of a redirect.
    if resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json")) {
        let v: Value = resp.json().await?;
        let direct = ["public_url", "direct_url", "url"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|u| u.as_str()).filter(|u| u.starts_with("http")))
            .ok_or_else(|| anyhow!("Thingiverse didn't say where the file is"))?
            .to_string();
        return super::download_to(client, &direct, dest).await;
    }
    super::save_response(resp, dest).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_from_api() {
        let link = ParsedLink { source: Source::Thingiverse, model_id: "763622".into(), profile_id: None, china: false, url: "https://www.thingiverse.com/thing:763622".into() };
        let thing = json!({
            "id": 763622, "name": "Benchy", "creator": {"name": "maker"},
            "thumbnail": "https://cdn.thingiverse.com/thumb.jpg",
            "default_image": {"sizes": [
                {"type": "thumb", "size": "small", "url": "https://cdn.thingiverse.com/s.jpg"},
                {"type": "display", "size": "large", "url": "https://cdn.thingiverse.com/l.jpg"}
            ]}
        });
        let files = json!([
            {"id": 1, "name": "boat.STL", "size": 1234, "thumbnail": "https://cdn.thingiverse.com/f.png"},
            {"id": 2, "name": "source.scad", "size": 10},
            {"id": 3, "name": "project.3mf", "size": 99, "thumbnail": "https://cdn.thingiverse.com/assets/default.png"}
        ]);
        let m = to_model(&link, &thing, &files);
        assert_eq!(m.title, "Benchy");
        assert_eq!(m.author.as_deref(), Some("maker"));
        assert_eq!(m.cover.as_deref(), Some("https://cdn.thingiverse.com/l.jpg"));
        assert_eq!(m.files.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["3", "1"]);
        assert_eq!(m.files[1].kind, "stl");
        assert_eq!(m.files[0].thumbnail, None);
        assert!(m.notes[0].contains("1 file"));
    }
}
