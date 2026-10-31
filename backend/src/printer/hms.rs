//! What HMS codes mean, and where Bambu's wiki explains them.
//!
//! Texts come from Bambu's own list (`e.bambulab.com/query.php?lang=…`, the
//! one Bambu Studio uses), fetched in English and Swedish, cached in
//! `data/cache/hms_<lang>.json` and refreshed weekly. Wiki pages can't be
//! derived from a code (the path differs by printer family and AMS codes use a
//! base code), so `hms_wiki.json` maps codes to pages: taken from ha-bambulab
//! (MIT License, Copyright (c) 2023 ha-bambulab contributors), reduced to one
//! page per code.

use crate::state::Shared;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
    time::Duration,
};

const LANGS: [&str; 2] = ["en", "sv"];
const REFRESH: Duration = Duration::from_secs(7 * 86_400);
const WIKI_HOME: &str = "https://wiki.bambulab.com/en/hms/home";

/// Language → (16-digit code → text).
static TEXTS: RwLock<Option<HashMap<&'static str, HashMap<String, String>>>> = RwLock::new(None);

fn wiki() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| serde_json::from_str(include_str!("hms_wiki.json")).unwrap_or_default())
}

/// "0700_2300_0002_0025" → "0700230000020025".
fn key(code: &str) -> String {
    code.replace('_', "").to_uppercase()
}

/// The wiki page for a code, or Bambu's HMS overview when it has none.
pub fn wiki_url(code: &str) -> String {
    match wiki().get(&key(code)).and_then(|v| v.split_once('|')) {
        Some((prefix, page)) => format!("https://wiki.bambulab.com{prefix}/troubleshooting/hmscode/{page}"),
        None => WIKI_HOME.to_string(),
    }
}

/// The printer's own words for a code, per language (when known).
pub fn texts(code: &str) -> HashMap<&'static str, String> {
    let k = key(code);
    let g = TEXTS.read().unwrap_or_else(|e| e.into_inner());
    let mut out = HashMap::new();
    if let Some(all) = g.as_ref() {
        for (lang, map) in all {
            if let Some(t) = map.get(&k).filter(|t| !t.trim().is_empty()) {
                out.insert(*lang, t.trim().to_string());
            }
        }
    }
    out
}

/// Codes and texts from Bambu's answer (`data.device_hms.<lang>`).
fn parse(v: &Value, lang: &str) -> HashMap<String, String> {
    v.pointer(&format!("/data/device_hms/{lang}"))
        .and_then(|l| l.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e.get("ecode")?.as_str()?.to_uppercase(), e.get("intro")?.as_str()?.to_string())))
        .filter(|(c, t)| c.len() == 16 && !t.is_empty())
        .collect()
}

fn install(lang: &'static str, map: HashMap<String, String>) {
    if map.is_empty() {
        return;
    }
    TEXTS.write().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(lang, map);
}

/// Load the cached lists, then keep them fresh. Without internet the cache
/// (or nothing: the code and wiki link still show) is used.
pub async fn keep_fresh(state: Shared) {
    let dir = state.cfg.data_dir.join("cache");
    for lang in LANGS {
        if let Ok(b) = tokio::fs::read(dir.join(format!("hms_{lang}.json"))).await
            && let Ok(v) = serde_json::from_slice::<Value>(&b)
        {
            install(lang, parse(&v, lang));
        }
    }
    loop {
        for lang in LANGS {
            let path = dir.join(format!("hms_{lang}.json"));
            let age = tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a < REFRESH) {
                continue;
            }
            let fetched = async {
                let resp = state.http.get(format!("https://e.bambulab.com/query.php?lang={lang}")).timeout(Duration::from_secs(30)).send().await?.error_for_status()?;
                anyhow::Ok(resp.bytes().await?)
            }
            .await;
            match fetched.map(|b| (serde_json::from_slice::<Value>(&b), b)) {
                Ok((Ok(v), bytes)) => {
                    let map = parse(&v, lang);
                    if map.len() > 100 {
                        tracing::info!("loaded {} HMS texts ({lang})", map.len());
                        install(lang, map);
                        let _ = tokio::fs::create_dir_all(&dir).await;
                        let _ = tokio::fs::write(&path, &bytes).await;
                    }
                }
                Ok((Err(e), _)) => tracing::warn!("Bambu's HMS list ({lang}) didn't parse: {e}"),
                Err(e) => tracing::debug!("couldn't fetch Bambu's HMS list ({lang}): {e:#}"),
            }
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_pages_from_the_map() {
        // An AMS code: AMS A, slot 4 → the base code's page.
        assert_eq!(wiki_url("0700_2300_0002_0025"), "https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0700_2000_0002_0025");
        assert_eq!(wiki_url("0300_0100_0001_0003"), "https://wiki.bambulab.com/en/x1/troubleshooting/hmscode/0300_0100_0001_0003");
        assert_eq!(wiki_url("FFFF_FFFF_FFFF_FFFF"), WIKI_HOME);
    }

    #[test]
    fn reads_bambus_list() {
        let v = serde_json::json!({"data": {"device_hms": {"ver": 1, "en": [
            {"ecode": "0700230000020025", "intro": "AMS A slot 4 feed resistance is too high."},
            {"ecode": "0500060000020045", "intro": ""}
        ]}}});
        let m = parse(&v, "en");
        assert_eq!(m.len(), 1);
        install("en", m);
        assert_eq!(texts("0700_2300_0002_0025").get("en").map(String::as_str), Some("AMS A slot 4 feed resistance is too high."));
    }
}
