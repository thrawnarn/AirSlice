//! MakerWorld, through Bambu's API backend (api.bambulab.com).
//!
//! Model metadata is public. Downloading a print profile's 3MF needs a Bambu
//! account token, which the user links once in Settings. Only the slicer
//! side (this server) talks to Bambu; the printer itself stays offline.

use super::ParsedLink;
use crate::models::{FilamentInfo, ModelFile, ResolvedModel, Source};
use anyhow::{Context, anyhow, bail};
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub fn api_base(china: bool) -> &'static str {
    if china { "https://api.bambulab.cn" } else { "https://api.bambulab.com" }
}

async fn get_json(client: &reqwest::Client, url: &str, token: Option<&str>) -> anyhow::Result<Value> {
    let mut req = client.get(url).header("Referer", "https://makerworld.com/").header("Accept", "application/json");
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| {
        tracing::warn!("MakerWorld request failed: {e:#}");
        anyhow!("Couldn't reach MakerWorld. Check that the AirSlice server has internet access.")
    })?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    match status {
        s if s.is_success() => serde_json::from_str(&body).context("MakerWorld returned something unexpected"),
        StatusCode::UNAUTHORIZED => bail!("Your Bambu account sign-in has expired. Sign in again under Settings → Model sites."),
        StatusCode::FORBIDDEN => bail!(
            "MakerWorld refused access{} — the model may need points, a purchase, or be region-locked.",
            upstream_message(&body).map(|m| format!(" ({m})")).unwrap_or_default()
        ),
        StatusCode::NOT_FOUND => bail!("Model not found on MakerWorld."),
        StatusCode::IM_A_TEAPOT => bail!(
            "MakerWorld is asking this server to solve a CAPTCHA{}. It usually clears within a few hours.",
            upstream_message(&body).map(|m| format!(" ({m})")).unwrap_or_default()
        ),
        StatusCode::TOO_MANY_REQUESTS => bail!("MakerWorld is rate-limiting requests. Try again in a minute."),
        s => bail!("MakerWorld returned HTTP {s}"),
    }
}

fn upstream_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error").or_else(|| v.get("message")).and_then(|m| m.as_str()).map(str::to_string)
}

/// A browser-friendly version of a MakerWorld picture. The CDN serves the
/// originals (often several MB) as `application/octet-stream` with
/// `nosniff`, which browsers won't show; a resize parameter makes it send a
/// small WebP with the right type.
pub fn thumb(url: String) -> String {
    if url.contains("bblmw.com/") && !url.contains('?') { format!("{url}?x-oss-process=image/resize,w_{THUMB_WIDTH}/format,webp") } else { url }
}

const THUMB_WIDTH: u32 = 480;

fn s(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Printer names from MakerWorld's loosely-typed compatibility blocks.
fn printer_names(v: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |x: &Value| {
        let name = s(x.get("devProductName"))
            .or_else(|| s(x.get("devModelName")))
            .or_else(|| s(x.get("name")))
            .or_else(|| x.as_str().map(str::to_string));
        if let Some(n) = name
            && !out.contains(&n) {
                out.push(n);
            }
    };
    match v {
        Some(Value::Array(items)) => items.iter().for_each(&mut push),
        Some(obj @ Value::Object(_)) => push(obj),
        Some(Value::String(_)) => push(v.unwrap()),
        _ => {}
    }
    out
}

fn filaments(v: Option<&Value>) -> Vec<FilamentInfo> {
    v.and_then(|f| f.as_array())
        .map(|items| {
            items
                .iter()
                .map(|f| FilamentInfo {
                    kind: s(f.get("type")).or_else(|| s(f.get("filamentType"))).unwrap_or_default(),
                    color: s(f.get("color")).or_else(|| s(f.get("filamentColor"))).map(|c| {
                        if c.starts_with('#') { c.chars().take(7).collect() } else { format!("#{}", c.chars().take(6).collect::<String>()) }
                    }).unwrap_or_default(),
                })
                .filter(|f| !f.kind.is_empty() || !f.color.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn resolve(client: &reqwest::Client, link: &ParsedLink, token: Option<&str>) -> anyhow::Result<ResolvedModel> {
    let base = format!("{}/v1/design-service", api_base(link.china));
    let design = get_json(client, &format!("{base}/design/{}", link.model_id), token).await?;
    let hits = get_json(client, &format!("{base}/design/{}/instances", link.model_id), token)
        .await
        .ok()
        .and_then(|v| v.get("hits").cloned())
        .and_then(|h| h.as_array().cloned())
        .unwrap_or_default();
    let design_instances = design.get("instances").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    // Prefer /instances (richer), fall back to the design's own list.
    let instances = if hits.is_empty() { design_instances.clone() } else { hits };

    let mut files = Vec::new();
    for inst in &instances {
        let Some(profile_id) = s(inst.get("profileId")) else { continue };
        let inst_id = s(inst.get("id"));
        // Printer compatibility lives on the design's copy of the instance.
        let detail = design_instances.iter().find(|d| s(d.get("id")) == inst_id).unwrap_or(inst);
        let model_info = detail.pointer("/extention/modelInfo").or_else(|| detail.pointer("/extension/modelInfo"));
        let made_for = printer_names(model_info.and_then(|m| m.get("compatibility")));
        let also = printer_names(model_info.and_then(|m| m.get("otherCompatibility")));
        let plates = inst
            .get("plates")
            .or_else(|| detail.get("plates"))
            .and_then(|p| p.as_array())
            .map(|p| p.len() as u32)
            .filter(|n| *n > 0)
            .or_else(|| inst.get("plateCount").and_then(|n| n.as_u64()).map(|n| n as u32));
        // The design's copy of the instance has these; the list may not.
        let prediction_secs = [inst, detail].iter().find_map(|v| v.get("prediction").and_then(|n| n.as_u64()).filter(|n| *n > 0));
        let weight_g = [inst, detail].iter().find_map(|v| v.get("weight").and_then(|n| n.as_f64()).filter(|n| *n > 0.0));
        let mut fil = filaments(inst.get("instanceFilaments"));
        if fil.is_empty() {
            fil = filaments(detail.get("instanceFilaments"));
        }
        files.push(ModelFile {
            id: profile_id,
            name: s(inst.get("title")).unwrap_or_else(|| "Print profile".into()),
            kind: "profile".into(),
            size_bytes: None,
            thumbnail: s(inst.get("cover")).or_else(|| s(detail.get("cover"))).map(thumb),
            printer: made_for.first().cloned(),
            compatible: made_for.into_iter().skip(1).chain(also).collect(),
            plates,
            need_ams: inst.get("needAms").and_then(|v| v.as_bool()),
            filaments: fil,
            download_type: None,
            folder: None,
            prediction_secs,
            weight_g,
        });
    }

    let mut notes = Vec::new();
    if files.is_empty() {
        notes.push("This model has no print profiles on MakerWorld.".into());
    }
    if token.is_none() {
        notes.push("Link your Bambu account under Settings → Model sites to download MakerWorld files.".into());
    }
    let preselected = link.profile_id.clone().filter(|p| files.iter().any(|f| &f.id == p)).or_else(|| {
        // MakerWorld's default profile
        instances
            .iter()
            .find(|i| i.get("isDefault").and_then(|d| d.as_bool()) == Some(true))
            .and_then(|i| s(i.get("profileId")))
    });

    Ok(ResolvedModel {
        source: Source::Makerworld,
        model_id: link.model_id.clone(),
        url: link.url.clone(),
        title: s(design.get("title")).unwrap_or_else(|| format!("MakerWorld model {}", link.model_id)),
        author: s(design.pointer("/designCreator/name")),
        cover: s(design.get("coverUrl")).or_else(|| s(design.get("cover"))).map(thumb),
        files,
        preselected_file_id: preselected,
        notes,
        internal_id: s(design.get("modelId")),
        details: Some(details(&design)),
    })
}

// ------------------------------------------------------------------ collections

/// A MakerWorld collection: the owner's own, one they liked, or their downloads.
#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    /// The collection id, or "downloads".
    pub id: String,
    pub title: String,
    pub count: Option<u64>,
    /// Up to four model pictures.
    pub covers: Vec<String>,
    /// "mine", "liked" or "downloads".
    pub kind: &'static str,
    /// Who made it, for liked collections.
    pub owner: Option<String>,
}

/// One model in a collection.
#[derive(Debug, Clone, Serialize)]
pub struct CollectionModel {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub author: Option<String>,
    pub url: String,
    pub prints: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionPage {
    pub total: u64,
    pub models: Vec<CollectionModel>,
}

fn collection(v: &Value, kind: &'static str) -> Option<Collection> {
    let covers = match s(v.get("cover")) {
        Some(c) => vec![thumb(c)],
        None => v.get("designCover").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| s(Some(c))).take(4).map(thumb).collect(),
    };
    Some(Collection {
        id: s(v.get("id"))?,
        title: s(v.get("titleTranslated")).or_else(|| s(v.get("title"))).unwrap_or_else(|| "Collection".into()),
        count: v.get("designCnt").and_then(|n| n.as_u64()),
        covers,
        kind,
        owner: s(v.pointer("/creator/name")).filter(|_| kind == "liked"),
    })
}

/// The signed-in account's collections: its own, the ones it liked, then
/// its downloads (MakerWorld keeps those like a collection).
pub async fn collections(client: &reqwest::Client, china: bool, token: &str) -> anyhow::Result<Vec<Collection>> {
    let base = format!("{}/v1/design-service", api_base(china));
    let mine = get_json(client, &format!("{base}/my/favorites/listlite?limit=100&offset=0"), Some(token)).await?;
    let mut out: Vec<Collection> = mine.get("hits").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|v| collection(v, "mine")).collect();
    // The rest is extra: a failure there shouldn't hide the owner's own.
    if let Ok(liked) = get_json(client, &format!("{base}/my/favorites/v2/like?limit=100&offset=0"), Some(token)).await {
        out.extend(liked.get("hits").and_then(|h| h.as_array()).into_iter().flatten().filter_map(|v| collection(v, "liked")));
    }
    if let Ok(dl) = get_json(client, &format!("{base}/my/favorites/download/designs?limit=4&offset=0"), Some(token)).await {
        let hits = dl.get("hits").and_then(|h| h.as_array()).cloned().unwrap_or_default();
        if !hits.is_empty() {
            out.push(Collection {
                id: "downloads".into(),
                title: "Downloads".into(),
                count: dl.get("total").and_then(|n| n.as_u64()),
                covers: hits.iter().filter_map(|h| s(h.get("cover"))).take(4).map(thumb).collect(),
                kind: "downloads",
                owner: None,
            });
        }
    }
    Ok(out)
}

/// A page of models in a collection (`id` from `collections`).
pub async fn collection_models(client: &reqwest::Client, china: bool, token: &str, id: &str, offset: u32, limit: u32) -> anyhow::Result<CollectionPage> {
    let base = format!("{}/v1/design-service", api_base(china));
    let limit = limit.clamp(1, 50);
    let url = if id == "downloads" {
        format!("{base}/my/favorites/download/designs?limit={limit}&offset={offset}")
    } else {
        if !id.chars().all(|c| c.is_ascii_digit()) {
            bail!("unknown collection {id:?}");
        }
        format!("{base}/favorites/{id}/designs?limit={limit}&offset={offset}")
    };
    let v = get_json(client, &url, Some(token)).await?;
    let site = if china { "https://makerworld.com.cn" } else { "https://makerworld.com" };
    let models: Vec<CollectionModel> = v
        .get("hits")
        .and_then(|h| h.as_array())
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let id = s(h.get("id"))?;
            Some(CollectionModel {
                url: format!("{site}/en/models/{id}"),
                title: s(h.get("titleTranslated")).or_else(|| s(h.get("title"))).unwrap_or_else(|| format!("Model {id}")),
                cover: s(h.get("cover")).map(thumb),
                author: s(h.pointer("/designCreator/name")),
                prints: h.get("printCount").and_then(|n| n.as_u64()),
                id,
            })
        })
        .collect();
    let total = v.get("total").and_then(|n| n.as_u64()).unwrap_or(models.len() as u64);
    Ok(CollectionPage { total, models })
}

/// What a model page shows: description (translated when MakerWorld has
/// one), pictures, tags and counts.
fn details(d: &Value) -> crate::models::ModelDetails {
    let text = |k: &str| s(d.get(k)).filter(|t| !t.trim().is_empty());
    let mut images: Vec<String> = Vec::new();
    for v in [d.get("coverUrl"), d.get("coverLandscape")].into_iter().flatten() {
        if let Some(u) = s(Some(v)) {
            images.push(u);
        }
    }
    for key in ["design_pictures", "real_pictures"] {
        for p in d.pointer(&format!("/designExtension/{key}")).and_then(|p| p.as_array()).into_iter().flatten() {
            if let Some(u) = s(p.get("url")) {
                images.push(u);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    images.retain(|u| seen.insert(u.clone()));
    images.truncate(30);
    let mut description = text("summaryTranslated").or_else(|| text("summary")).map(|h| super::html::to_blocks(&h)).unwrap_or_default();
    // The maker's own footer (printing tips and the like) follows.
    if let Some(f) = d.pointer("/designCreatorCustomization/designFooterTranslated").and_then(|f| f.as_str()).filter(|f| !f.trim().is_empty())
        .or_else(|| d.pointer("/designCreatorCustomization/designFooter").and_then(|f| f.as_str()))
    {
        description.extend(super::html::to_blocks(f));
    }
    let list = |k: &str| -> Vec<String> {
        let v = d.get(format!("{k}Translated")).and_then(|t| t.as_array()).filter(|a| !a.is_empty()).or_else(|| d.get(k).and_then(|t| t.as_array()));
        v.into_iter().flatten().filter_map(|t| s(Some(t))).collect()
    };
    crate::models::ModelDetails {
        summary: None,
        description,
        images: images.into_iter().map(thumb).collect(),
        tags: list("tags"),
        categories: d.get("categories").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| s(c.get("name"))).collect(),
        likes: d.get("likeCount").and_then(|n| n.as_u64()),
        downloads: d.get("downloadCount").and_then(|n| n.as_u64()),
        prints: d.get("printCount").and_then(|n| n.as_u64()),
        collections: d.get("collectionCount").and_then(|n| n.as_u64()),
        license: text("license"),
        published_at: text("createTime").and_then(|t| t.parse::<jiff::Timestamp>().ok()).map(|t| t.as_second()),
    }
}

/// Download a print profile's 3MF.
pub async fn download(
    client: &reqwest::Client,
    china: bool,
    internal_model_id: &str,
    profile_id: &str,
    token: &str,
    dest: &Path,
) -> anyhow::Result<(u64, Option<String>)> {
    let url = format!(
        "{}/v1/iot-service/api/user/profile/{}?model_id={}",
        api_base(china),
        profile_id,
        urlencode(internal_model_id)
    );
    let manifest = get_json(client, &url, Some(token)).await?;
    let signed = manifest
        .get("url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| anyhow!("MakerWorld did not return a download link"))?;
    // The signed CDN link is used as-is; re-encoding it breaks the signature.
    let size = super::download_to(client, signed, dest).await?;
    Ok((size, manifest.get("name").and_then(|n| n.as_str()).map(str::to_string)))
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_get_a_browser_friendly_form() {
        let u = "https://makerworld.bblmw.com/makerworld/model/X/design/a.jpg".to_string();
        assert!(thumb(u.clone()).ends_with("a.jpg?x-oss-process=image/resize,w_480/format,webp"));
        assert_eq!(thumb(thumb(u.clone())), thumb(u), "only once");
        assert_eq!(thumb("https://media.printables.com/x.png".into()), "https://media.printables.com/x.png");
    }

    #[test]
    fn reads_model_details() {
        let d = serde_json::json!({
            "coverUrl": "https://makerworld.bblmw.com/c.jpg",
            "summary": "<p>Original</p>", "summaryTranslated": "<p>Translated <b>text</b></p><ul><li>One</li></ul>",
            "designCreatorCustomization": {"designFooter": "<p>Tips</p>"},
            "designExtension": {"design_pictures": [{"url": "https://makerworld.bblmw.com/c.jpg"}, {"url": "https://makerworld.bblmw.com/d.jpg"}]},
            "tags": ["box"], "tagsTranslated": [], "categories": [{"name": "Organizers"}],
            "likeCount": 459, "printCount": 299, "downloadCount": 591, "collectionCount": 946,
            "license": "Standard Digital File License", "createTime": "2024-06-30T10:11:03Z"
        });
        let x = details(&d);
        assert_eq!(x.description.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["Translated text", "One", "Tips"]);
        assert_eq!(x.images.len(), 2, "cover once");
        assert!(x.images[0].contains("x-oss-process"));
        assert_eq!((x.tags, x.categories), (vec!["box".to_string()], vec!["Organizers".to_string()]));
        assert_eq!((x.likes, x.prints, x.downloads, x.collections), (Some(459), Some(299), Some(591), Some(946)));
        assert_eq!(x.published_at, Some(1719742263));
    }

    #[test]
    fn reads_collections() {
        let mine = serde_json::json!({"id": 5785457, "title": "Albin", "designCnt": 5, "cover": "",
            "designCover": ["https://makerworld.bblmw.com/a.png", "https://makerworld.bblmw.com/b.png"],
            "creator": {"name": "me"}});
        let c = collection(&mine, "mine").unwrap();
        assert_eq!((c.id.as_str(), c.title.as_str(), c.count, c.covers.len(), c.owner), ("5785457", "Albin", Some(5), 2, None));
        let liked = serde_json::json!({"id": 1, "title": "Board games", "cover": "https://makerworld.bblmw.com/c.png", "creator": {"name": "felipone"}});
        let c = collection(&liked, "liked").unwrap();
        assert_eq!((c.covers.len(), c.owner.as_deref()), (1, Some("felipone")));
    }
    use serde_json::json;

    #[test]
    fn compatibility_shapes() {
        assert_eq!(printer_names(Some(&json!({"devProductName": "A1", "devModelName": "N2S"}))), vec!["A1"]);
        assert_eq!(
            printer_names(Some(&json!([{"devProductName": "P1S"}, {"devModelName": "X1C"}, {"devProductName": "P1S"}]))),
            vec!["P1S", "X1C"]
        );
        assert!(printer_names(None).is_empty());
    }

    #[test]
    fn filament_colors() {
        let f = filaments(Some(&json!([{"type": "PLA", "color": "FF0000FF"}, {"type": "PETG", "color": "#00FF00"}])));
        assert_eq!(f[0], FilamentInfo { kind: "PLA".into(), color: "#FF0000".into() });
        assert_eq!(f[1].color, "#00FF00");
    }
}
