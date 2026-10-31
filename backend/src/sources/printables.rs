//! Printables (printables.com) via its public GraphQL API.

use super::{SLICEABLE, ParsedLink, extension};
use crate::models::{ModelFile, ResolvedModel, Source};
use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};

const API: &str = "https://api.printables.com/graphql/";
const MEDIA: &str = "https://media.printables.com/";

const FILES_QUERY: &str = r#"
query ModelFiles($id: ID!) {
  model: print(id: $id) {
    id
    filesType
    stls { id name fileSize filePreviewPath folder }
    gcodes { id name fileSize filePreviewPath folder }
    slas { id name fileSize folder }
    otherFiles { id name fileSize folder }
  }
}"#;

// Kept separate so a schema change in these optional fields can't break
// the file listing.
const META_QUERY: &str = r#"
query ModelMeta($id: ID!) {
  model: print(id: $id) { id name user { publicUsername } image { filePath } }
}"#;

// The model page's extras; a failure here only leaves them out.
const DETAILS_QUERY: &str = r#"
query ModelDetails($id: ID!) {
  model: print(id: $id) {
    id summary description likesCount downloadCount makesCount datePublished
    license { name } tags { name } category { name } images { filePath }
  }
}"#;

const DOWNLOAD_MUTATION: &str = r#"
mutation GetDownloadLink($id: ID!, $modelId: ID!, $fileType: DownloadFileTypeEnum!, $source: DownloadSourceEnum!) {
  getDownloadLink(id: $id, printId: $modelId, fileType: $fileType, source: $source) {
    ok
    errors { field messages }
    output { link }
  }
}"#;

async fn gql(client: &reqwest::Client, op: &str, query: &str, vars: Value) -> anyhow::Result<Value> {
    let resp = client
        .post(API)
        .header("Origin", "https://www.printables.com")
        .header("Referer", "https://www.printables.com/")
        .json(&json!({ "operationName": op, "query": query, "variables": vars }))
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("Printables request failed: {e:#}");
            anyhow!("Couldn't reach Printables. Check that the AirSlice server has internet access.")
        })?;
    let status = resp.status();
    let body: Value = resp.json().await.with_context(|| format!("Printables returned HTTP {status}"))?;
    if let Some(errs) = body.get("errors").and_then(|e| e.as_array()).filter(|e| !e.is_empty()) {
        let msg = errs.iter().filter_map(|e| e.get("message")?.as_str()).collect::<Vec<_>>().join("; ");
        if body.get("data").map(|d| d.is_null()).unwrap_or(true) {
            bail!("Printables: {msg}");
        }
        tracing::warn!("Printables {op}: {msg}");
    }
    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}

fn media(path: Option<&str>) -> Option<String> {
    path.filter(|p| !p.is_empty()).map(|p| {
        if p.starts_with("http") { p.to_string() } else { format!("{MEDIA}{}", p.trim_start_matches('/')) }
    })
}

pub async fn resolve(client: &reqwest::Client, link: &ParsedLink) -> anyhow::Result<ResolvedModel> {
    let vars = json!({ "id": link.model_id });
    let files = gql(client, "ModelFiles", FILES_QUERY, vars.clone()).await?;
    let model = files.get("model").filter(|m| !m.is_null()).ok_or_else(|| anyhow!("Model not found on Printables."))?;
    let meta = gql(client, "ModelMeta", META_QUERY, vars).await.unwrap_or(Value::Null);
    let meta = meta.get("model").cloned().unwrap_or(Value::Null);
    let details = gql(client, "ModelDetails", DETAILS_QUERY, json!({ "id": link.model_id })).await.ok().and_then(|d| d.get("model").cloned()).filter(|m| !m.is_null()).map(|m| details(&m));

    let mut out = Vec::new();
    let mut skipped = 0;
    for (list, dl_type) in [("stls", "stl"), ("otherFiles", "other"), ("gcodes", "gcode"), ("slas", "sla")] {
        for f in model.get(list).and_then(|l| l.as_array()).into_iter().flatten() {
            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let id = match f.get("id") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
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
                size_bytes: f.get("fileSize").and_then(|s| s.as_u64()),
                thumbnail: media(f.get("filePreviewPath").and_then(|p| p.as_str())),
                printer: None,
                compatible: vec![],
                plates: None,
                need_ams: None,
                filaments: vec![],
                download_type: Some(dl_type.to_string()),
                folder: f.get("folder").and_then(|p| p.as_str()).filter(|s| !s.is_empty()).map(str::to_string),
                prediction_secs: None,
                weight_g: None,
            });
        }
    }
    // 3MF projects first: they carry print settings and plate layout.
    out.sort_by_key(|f| (f.kind != "3mf", f.folder.clone(), f.name.to_lowercase()));

    let mut notes = Vec::new();
    if out.is_empty() {
        notes.push("This model has no files that can be sliced (STL, 3MF, OBJ, STEP).".into());
    } else if skipped > 0 {
        notes.push(format!("{skipped} file(s) hidden because they can't be sliced (e.g. G-code for other printers)."));
    }

    Ok(ResolvedModel {
        source: Source::Printables,
        model_id: link.model_id.clone(),
        url: link.url.clone(),
        title: meta.get("name").and_then(|n| n.as_str()).map(str::to_string).unwrap_or_else(|| format!("Printables model {}", link.model_id)),
        author: meta.pointer("/user/publicUsername").and_then(|n| n.as_str()).map(str::to_string),
        cover: media(meta.pointer("/image/filePath").and_then(|p| p.as_str())),
        files: out,
        preselected_file_id: None,
        notes,
        internal_id: None,
        details,
    })
}

fn details(m: &Value) -> crate::models::ModelDetails {
    let text = |k: &str| m.get(k).and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.trim().is_empty());
    let names = |k: &str| -> Vec<String> {
        match m.get(k) {
            Some(Value::Array(a)) => a.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect(),
            Some(o @ Value::Object(_)) => o.get("name").and_then(|n| n.as_str()).map(|n| vec![n.to_string()]).unwrap_or_default(),
            _ => vec![],
        }
    };
    crate::models::ModelDetails {
        summary: text("summary").map(|s| super::html::decode(&s)),
        description: text("description").map(|d| super::html::to_blocks(&d)).unwrap_or_default(),
        images: m.get("images").and_then(|i| i.as_array()).into_iter().flatten().filter_map(|i| media(i.get("filePath").and_then(|p| p.as_str()))).take(30).collect(),
        tags: names("tags"),
        categories: names("category"),
        likes: m.get("likesCount").and_then(|n| n.as_u64()),
        downloads: m.get("downloadCount").and_then(|n| n.as_u64()),
        prints: m.get("makesCount").and_then(|n| n.as_u64()),
        collections: None,
        license: m.pointer("/license/name").and_then(|n| n.as_str()).map(str::to_string),
        published_at: text("datePublished").and_then(|d| jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%S%.f%:z", &d).ok().or_else(|| d.parse::<jiff::Timestamp>().ok())).map(|t| t.as_second()),
    }
}

pub async fn download_url(client: &reqwest::Client, model_id: &str, file: &ModelFile) -> anyhow::Result<String> {
    let data = gql(
        client,
        "GetDownloadLink",
        DOWNLOAD_MUTATION,
        json!({
            "id": file.id,
            "modelId": model_id,
            "fileType": file.download_type.as_deref().unwrap_or("stl"),
            "source": "model_detail",
        }),
    )
    .await?;
    let res = data.get("getDownloadLink").ok_or_else(|| anyhow!("Printables did not return a download link"))?;
    if res.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let msg = res
            .get("errors")
            .and_then(|e| e.as_array())
            .map(|e| e.iter().flat_map(|x| x.get("messages").and_then(|m| m.as_array()).cloned().unwrap_or_default()).filter_map(|m| m.as_str().map(str::to_string)).collect::<Vec<_>>().join("; "))
            .unwrap_or_default();
        bail!("Printables refused the download{}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
    }
    res.pointer("/output/link")
        .and_then(|l| l.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Printables did not return a download link"))
}
