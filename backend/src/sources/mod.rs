//! Model sites we can import from.

pub mod html;
pub mod makerworld;
pub mod printables;
pub mod thingiverse;

use crate::models::{ModelFile, ResolvedModel, Source};
use anyhow::{Context, anyhow, bail};
use futures::StreamExt;
use std::path::Path;
use tokio::io::AsyncWriteExt;

pub const USER_AGENT: &str = concat!("AirSlice/", env!("CARGO_PKG_VERSION"), " (+self-hosted)");
/// Upper bound for a downloaded model.
pub const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// File extensions OrcaSlicer can open.
pub const SLICEABLE: &[&str] = &["3mf", "stl", "obj", "step", "stp"];

pub fn extension(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".gcode.3mf") {
        return "gcode.3mf".into();
    }
    Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("").to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLink {
    pub source: Source,
    pub model_id: String,
    /// MakerWorld profile id from `#profileId-123`.
    pub profile_id: Option<String>,
    pub china: bool,
    pub url: String,
}

/// Find a supported model link in arbitrary text (share sheets often send
/// "Check out this model! https://…").
pub fn parse_link(input: &str) -> anyhow::Result<ParsedLink> {
    let candidate = input
        .split_whitespace()
        .find(|w| w.contains("makerworld.") || w.contains("printables.com") || w.contains("thingiverse.com"))
        .ok_or_else(|| anyhow!("Paste a MakerWorld, Printables or Thingiverse model link."))?;
    let candidate = candidate.trim_matches(|c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c == ')' || c == '(');
    let with_scheme =
        if candidate.starts_with("http") { candidate.to_string() } else { format!("https://{candidate}") };
    let url = url::Url::parse(&with_scheme).context("That doesn't look like a valid link.")?;
    let host = url.host_str().unwrap_or("").to_lowercase();
    let path = url.path();

    let digits_after = |marker: &str| -> Option<String> {
        let idx = path.find(marker)? + marker.len();
        let id: String = path[idx..].chars().take_while(|c| c.is_ascii_digit()).collect();
        (!id.is_empty()).then_some(id)
    };

    if host == "makerworld.com" || host.ends_with(".makerworld.com") || host.starts_with("makerworld.com.") {
        let model_id = digits_after("/models/").ok_or_else(|| anyhow!("Open a specific model on MakerWorld and share that link."))?;
        let profile_id = url.fragment().and_then(|f| {
            let f = f.to_lowercase();
            let idx = f.find("profileid")? + "profileid".len();
            let id: String = f[idx..].trim_start_matches(['-', '=']).chars().take_while(|c| c.is_ascii_digit()).collect();
            (!id.is_empty()).then_some(id)
        });
        return Ok(ParsedLink {
            source: Source::Makerworld,
            model_id: model_id.clone(),
            profile_id,
            china: host.ends_with(".cn"),
            url: format!("https://{host}/models/{model_id}"),
        });
    }
    if host == "printables.com" || host.ends_with(".printables.com") {
        let model_id = digits_after("/model/").ok_or_else(|| anyhow!("Open a specific model on Printables and share that link."))?;
        return Ok(ParsedLink {
            source: Source::Printables,
            model_id: model_id.clone(),
            profile_id: None,
            china: false,
            url: format!("https://www.printables.com/model/{model_id}"),
        });
    }
    if host == "thingiverse.com" || host.ends_with(".thingiverse.com") {
        let model_id = digits_after("/thing:").ok_or_else(|| anyhow!("Open a specific thing on Thingiverse and share that link."))?;
        return Ok(ParsedLink {
            source: Source::Thingiverse,
            model_id: model_id.clone(),
            profile_id: None,
            china: false,
            url: format!("https://www.thingiverse.com/thing:{model_id}"),
        });
    }
    bail!("Only MakerWorld, Printables and Thingiverse links are supported for now.")
}

/// Stream a URL to disk with a size cap. Returns bytes written.
pub async fn download_to(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<u64> {
    let resp = client.get(url).send().await.map_err(|e| {
        tracing::warn!("download failed: {e:#}");
        // A builder error means nothing was sent: the link itself is unusable.
        if e.is_builder() {
            anyhow!("Download failed — the model site gave an unusable download link.")
        } else {
            anyhow!("Download failed — couldn't reach the file host.")
        }
    })?;
    if !resp.status().is_success() {
        bail!("download failed: HTTP {}", resp.status());
    }
    save_response(resp, dest).await
}

/// Write a response body to disk with the size cap. Returns bytes written.
pub async fn save_response(resp: reqwest::Response, dest: &Path) -> anyhow::Result<u64> {
    if let Some(len) = resp.content_length()
        && len > MAX_DOWNLOAD {
            bail!("file is too large ({} MB)", len / 1_048_576);
        }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut written = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download interrupted")?;
        written += chunk.len() as u64;
        if written > MAX_DOWNLOAD {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            bail!("file is too large");
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(written)
}

/// Pick a file automatically (quick print): the preselected one, then one
/// made for this printer, then any 3MF, then the first sliceable file.
pub fn auto_pick<'a>(model: &'a ResolvedModel, printer_model: Option<&str>) -> Option<&'a ModelFile> {
    let printer_short = printer_model.map(|m| m.trim_start_matches("Bambu Lab ").to_lowercase());
    if let Some(id) = &model.preselected_file_id
        && let Some(f) = model.files.iter().find(|f| &f.id == id) {
            return Some(f);
        }
    if let Some(short) = &printer_short {
        let made_for = |f: &&ModelFile| {
            f.printer.as_deref().map(|p| p.to_lowercase().trim_start_matches("bambu lab ") == short).unwrap_or(false)
        };
        if let Some(f) = model.files.iter().find(made_for) {
            return Some(f);
        }
        let compatible = |f: &&ModelFile| {
            f.compatible.iter().any(|c| c.to_lowercase().trim_start_matches("bambu lab ") == short)
        };
        if let Some(f) = model.files.iter().find(compatible) {
            return Some(f);
        }
    }
    model
        .files
        .iter()
        .find(|f| f.kind == "profile" || f.kind == "3mf")
        .or_else(|| model.files.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makerworld_links() {
        let l = parse_link("https://makerworld.com/en/models/1400373-cute-thing?from=search#profileId-1452154").unwrap();
        assert_eq!(l.source, Source::Makerworld);
        assert_eq!(l.model_id, "1400373");
        assert_eq!(l.profile_id.as_deref(), Some("1452154"));
        assert!(!l.china);
        let l = parse_link("Look at this! makerworld.com/models/42").unwrap();
        assert_eq!(l.model_id, "42");
        assert_eq!(l.profile_id, None);
        let l = parse_link("https://makerworld.com.cn/zh/models/77").unwrap();
        assert!(l.china);
    }

    #[test]
    fn printables_links() {
        let l = parse_link("https://www.printables.com/model/3161-3d-benchy").unwrap();
        assert_eq!(l.source, Source::Printables);
        assert_eq!(l.model_id, "3161");
        let l = parse_link("https://www.printables.com/de/model/123456-x/files").unwrap();
        assert_eq!(l.model_id, "123456");
    }

    #[test]
    fn thingiverse_links() {
        let l = parse_link("Look: https://www.thingiverse.com/thing:763622/files").unwrap();
        assert_eq!(l.source, Source::Thingiverse);
        assert_eq!(l.model_id, "763622");
        assert_eq!(l.url, "https://www.thingiverse.com/thing:763622");
        assert_eq!(parse_link("thingiverse.com/thing:42").unwrap().model_id, "42");
    }

    #[test]
    fn rejects_others() {
        assert!(parse_link("https://cults3d.com/en/3d-model/x").is_err());
        assert!(parse_link("https://www.thingiverse.com/explore").is_err());
        assert!(parse_link("https://makerworld.com/en").is_err());
        assert!(parse_link("hello").is_err());
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("A.STL"), "stl");
        assert_eq!(extension("plate.gcode.3mf"), "gcode.3mf");
        assert_eq!(extension("x.3mf"), "3mf");
    }
}
