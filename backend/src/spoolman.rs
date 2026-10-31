//! Spoolman (optional): which spool sits in each tray, what is left on it,
//! what it cost, and booking the filament a print used.

use crate::{
    models::{Cost, CostSettings, SpoolmanSettings},
    printer::status::Tray,
    state::Shared,
};
use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// A spool as AirSlice shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Spool {
    pub id: i64,
    /// "Bambu Lab PLA Basic Black"
    pub name: String,
    pub material: String,
    /// `#RRGGBB`
    pub color: String,
    /// Grams left, when Spoolman knows.
    pub remaining_g: Option<f64>,
    /// Price per gram from the spool or its filament.
    #[serde(skip)]
    pub price_per_g: Option<f64>,
    /// RFID tag (`extra.tag`), upper case without quotes.
    #[serde(skip)]
    pub tag: Option<String>,
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64()).filter(|x| x.is_finite())
}

/// One spool from Spoolman's `/api/v1/spool` list. Spoolman leaves out keys
/// that are null.
pub fn parse_spool(v: &Value) -> Option<Spool> {
    let id = v.get("id")?.as_i64()?;
    let f = v.get("filament").cloned().unwrap_or(Value::Null);
    let vendor = f.pointer("/vendor/name").and_then(|x| x.as_str()).unwrap_or("");
    let fname = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
    let material = f.get("material").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // "Bambu Lab PLA Basic Black": the material only when the name lacks it.
    let with_material = if fname.to_lowercase().contains(&material.to_lowercase()) { "" } else { material.as_str() };
    let name = [vendor, with_material, fname].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    let color = f.get("color_hex").and_then(|x| x.as_str()).map(crate::threemf::hex_color).unwrap_or_default();
    // Price: the spool's own, else the filament's, per the weight it was sold with.
    let weight = num(v.get("initial_weight")).or_else(|| num(f.get("weight"))).filter(|w| *w > 0.0).unwrap_or(1000.0);
    let price_per_g = num(v.get("price")).or_else(|| num(f.get("price"))).filter(|p| *p > 0.0).map(|p| p / weight);
    let tag = v
        .pointer("/extra/tag")
        .and_then(|x| x.as_str())
        .map(|t| t.trim_matches('"').to_uppercase())
        .filter(|t| !t.is_empty());
    Some(Spool { id, name, material, color, remaining_g: num(v.get("remaining_weight")), price_per_g, tag })
}

static CACHE: Mutex<Option<(String, Instant, Vec<Spool>)>> = Mutex::new(None);
/// The last failure, so an unreachable Spoolman isn't waited for on every
/// job view (it's asked again after a minute, or when the settings change).
static FAILED: Mutex<Option<(String, Instant, String)>> = Mutex::new(None);

/// The active spools, cached for a minute.
pub async fn spools(state: &Shared, s: &SpoolmanSettings) -> anyhow::Result<Vec<Spool>> {
    let url = s.url.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        bail!("Spoolman isn't set up");
    }
    if let Some((u, at, list)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *u == url
        && at.elapsed() < Duration::from_secs(60)
    {
        return Ok(list.clone());
    }
    if let Some((u, at, why)) = FAILED.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *u == url
        && at.elapsed() < Duration::from_secs(60)
    {
        bail!("{why}");
    }
    let fetched: anyhow::Result<Vec<Value>> = async {
        state
            .http
            .get(format!("{url}/api/v1/spool"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .context("couldn't reach Spoolman")?
            .error_for_status()
            .context("Spoolman refused")?
            .json()
            .await
            .context("Spoolman sent something unexpected")
    }
    .await;
    let list = match fetched {
        Ok(l) => l,
        Err(e) => {
            *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = Some((url, Instant::now(), format!("{e:#}")));
            return Err(e);
        }
    };
    let spools: Vec<Spool> = list.iter().filter(|v| !v.get("archived").and_then(|a| a.as_bool()).unwrap_or(false)).filter_map(parse_spool).collect();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((url, Instant::now(), spools.clone()));
    Ok(spools)
}

pub fn forget_cache() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn link_key(printer_id: &str, tray: i32) -> String {
    format!("{printer_id}:{tray}")
}

/// The spool in each tray: by the spool's RFID tag, else a link made by hand.
pub fn tray_spools(printer_id: &str, trays: &[&Tray], spools: &[Spool], links: &std::collections::BTreeMap<String, i64>) -> HashMap<i32, Spool> {
    let mut out = HashMap::new();
    for t in trays {
        let by_tag = t.uuid.as_ref().and_then(|u| spools.iter().find(|s| s.tag.as_deref() == Some(u.to_uppercase().as_str())));
        let by_link = links.get(&link_key(printer_id, t.id)).and_then(|id| spools.iter().find(|s| s.id == *id));
        if let Some(s) = by_tag.or(by_link) {
            out.insert(t.id, s.clone());
        }
    }
    out
}

/// Spools in a printer's trays, or nothing when Spoolman isn't set up or
/// can't be reached (it's optional, so never an error for the caller).
pub async fn for_printer(state: &Shared, printer_id: &str) -> HashMap<i32, Spool> {
    let s = state.store.read(|d| d.spoolman.clone()).await;
    if s.url.trim().is_empty() {
        return HashMap::new();
    }
    let status = state.printers.status(printer_id).await;
    let trays = status.usable_trays();
    match spools(state, &s).await {
        Ok(list) => tray_spools(printer_id, &trays, &list, &s.links),
        Err(e) => {
            tracing::debug!("Spoolman: {e:#}");
            HashMap::new()
        }
    }
}

/// Price per gram for each tray: the linked spool's (its own price, else its
/// filament's), else the average of priced spools with the same material and
/// colour (a tray without RFID or link). Trays with neither are left out, so
/// the general price applies.
pub fn tray_prices(trays: &[&Tray], linked: &HashMap<i32, Spool>, spools: &[Spool]) -> HashMap<i32, f64> {
    let mut out = HashMap::new();
    for t in trays {
        if let Some(p) = linked.get(&t.id).and_then(|s| s.price_per_g) {
            out.insert(t.id, p);
            continue;
        }
        let same: Vec<f64> = spools
            .iter()
            .filter(|s| s.material.eq_ignore_ascii_case(t.kind.trim()) && !s.color.is_empty() && s.color.eq_ignore_ascii_case(&t.color))
            .filter_map(|s| s.price_per_g)
            .collect();
        if !same.is_empty() {
            out.insert(t.id, same.iter().sum::<f64>() / same.len() as f64);
        }
    }
    out
}

/// Spoolman prices for a printer's trays (empty when Spoolman is off).
pub async fn prices_for_printer(state: &Shared, printer_id: &str) -> HashMap<i32, f64> {
    let s = state.store.read(|d| d.spoolman.clone()).await;
    if s.url.trim().is_empty() {
        return HashMap::new();
    }
    let status = state.printers.status(printer_id).await;
    let trays = status.usable_trays();
    match spools(state, &s).await {
        Ok(list) => {
            let linked = tray_spools(printer_id, &trays, &list, &s.links);
            tray_prices(&trays, &linked, &list)
        }
        Err(_) => HashMap::new(),
    }
}

/// Filament per tray for a print: (tray, grams) from the slice and mapping.
pub fn use_per_tray(filaments: &[crate::models::SlicedFilament], mapping: &[i32], share: f64) -> Vec<(i32, f64)> {
    filaments
        .iter()
        .filter_map(|f| {
            let tray = *mapping.get(f.id as usize - 1)?;
            (tray >= 0 && f.used_g > 0.0).then_some((tray, f.used_g * share))
        })
        .collect()
}

/// Cost of grams per tray plus hours, with Spoolman prices per tray where
/// known and the general price per kg otherwise. `None` when there is no
/// price at all.
pub fn cost(use_g: &[(i32, f64)], prices: &HashMap<i32, f64>, hours: Option<f64>, c: &CostSettings, actual: bool) -> Option<Cost> {
    let fallback = c.price_per_kg.filter(|p| *p > 0.0).map(|p| p / 1000.0);
    let mut filament = Some(0.0);
    for (tray, g) in use_g {
        match prices.get(tray).copied().or(fallback) {
            Some(p) => filament = filament.map(|f| f + p * g),
            None => filament = None,
        }
    }
    let machine = match (c.per_hour.filter(|p| *p > 0.0), hours) {
        (Some(p), Some(h)) => Some(p * h),
        _ => None,
    };
    let any_price = fallback.is_some() || c.per_hour.is_some() || use_g.iter().any(|(t, _)| prices.contains_key(t));
    if !any_price || (filament.is_none() && machine.is_none()) {
        return None;
    }
    let round = |x: f64| (x * 100.0).round() / 100.0;
    Some(Cost {
        filament: filament.map(round),
        machine: machine.map(round),
        total: round(filament.unwrap_or(0.0) + machine.unwrap_or(0.0)),
        currency: c.currency.trim().to_string(),
        actual,
    })
}

/// Book what a print used on its spools (only when turned on).
pub async fn book(state: &Shared, s: &SpoolmanSettings, use_g: &[(i32, f64)], spools: &HashMap<i32, Spool>) -> Vec<String> {
    let url = s.url.trim().trim_end_matches('/');
    let mut done = Vec::new();
    for (tray, g) in use_g {
        let Some(spool) = spools.get(tray) else { continue };
        let res = state
            .http
            .put(format!("{url}/api/v1/spool/{}/use", spool.id))
            .timeout(Duration::from_secs(10))
            .json(&json!({ "use_weight": (g * 100.0).round() / 100.0 }))
            .send()
            .await
            .and_then(|r| r.error_for_status());
        match res {
            Ok(_) => done.push(format!("{g:.1} g from spool #{} ({})", spool.id, spool.name)),
            Err(e) => tracing::warn!("Spoolman: booking {g:.1} g on spool {} failed: {e}", spool.id),
        }
    }
    forget_cache();
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spool(id: i64, tag: Option<&str>, price: Option<f64>) -> Spool {
        Spool { id, name: format!("S{id}"), material: "PLA".into(), color: "#000000".into(), remaining_g: Some(500.0), price_per_g: price, tag: tag.map(str::to_string) }
    }

    #[test]
    fn parses_spoolman_spools() {
        let v = json!({"id": 6, "remaining_weight": 980.12, "initial_weight": 1000.0, "extra": {"tag": "\"bd71252f23c943ada1a3df7316160a0d\""},
            "filament": {"name": "PLA Basic", "material": "PLA", "color_hex": "000000", "price": 130.0, "weight": 1000.0, "vendor": {"name": "Bambu Lab"}}});
        let s = parse_spool(&v).unwrap();
        assert_eq!(s.name, "Bambu Lab PLA Basic");
        assert_eq!(parse_spool(&json!({"id": 2, "filament": {"name": "Matte Ivory White", "material": "PLA"}})).unwrap().name, "PLA Matte Ivory White");
        assert_eq!(s.tag.as_deref(), Some("BD71252F23C943ADA1A3DF7316160A0D"));
        assert_eq!(s.price_per_g, Some(0.13));
        assert_eq!(s.color, "#000000");
        // Spool price wins over the filament's; no price at all is None.
        let own = parse_spool(&json!({"id": 3, "price": 100.0, "initial_weight": 500.0, "filament": {"price": 130.0}})).unwrap();
        assert_eq!(own.price_per_g, Some(0.2));
        assert_eq!(parse_spool(&json!({"id": 1, "filament": {}})).unwrap().price_per_g, None);
    }

    #[test]
    fn finds_spools_by_tag_or_link() {
        let mk = |id: i32, uuid: Option<&str>| Tray {
            id, ams: Some(0), slot: Some(id as u32), kind: "PLA".into(), color: "#FFFFFF".into(), name: None, remain: None, spool_g: None, info_idx: None,
            uuid: uuid.map(str::to_string), active: false,
        };
        let (t0, t1, t2) = (mk(0, Some("abc")), mk(1, None), mk(2, None));
        let spools = vec![spool(10, Some("ABC"), None), spool(11, None, None)];
        let links = std::collections::BTreeMap::from([(link_key("p", 1), 11)]);
        let found = tray_spools("p", &[&t0, &t1, &t2], &spools, &links);
        assert_eq!(found[&0].id, 10);
        assert_eq!(found[&1].id, 11);
        assert!(!found.contains_key(&2));
    }

    #[test]
    fn prices_trays_from_spoolman() {
        let mk = |id: i32, kind: &str, color: &str| Tray {
            id, ams: Some(0), slot: Some(id as u32), kind: kind.into(), color: color.into(), name: None, remain: None, spool_g: None, info_idx: None, uuid: None, active: false,
        };
        let (t0, t1, t2) = (mk(0, "PLA", "#000000"), mk(1, "PLA", "#C12E1F"), mk(2, "PETG", "#FFFFFF"));
        let mut red_a = spool(3, None, Some(0.13));
        red_a.color = "#C12E1F".into();
        let mut red_b = spool(5, None, Some(0.11));
        red_b.color = "#C12E1F".into();
        let linked = HashMap::from([(0, spool(6, None, Some(0.2)))]);
        let prices = tray_prices(&[&t0, &t1, &t2], &linked, &[red_a, red_b]);
        assert_eq!(prices[&0], 0.2, "linked spool");
        assert!((prices[&1] - 0.12).abs() < 1e-9, "same material and colour, averaged");
        assert!(!prices.contains_key(&2), "no match: the general price applies");
    }

    #[test]
    fn works_out_cost() {
        let prices = HashMap::from([(0, 0.13)]);
        let c = CostSettings { currency: "kr".into(), price_per_kg: Some(150.0), per_hour: Some(1.5) };
        // 100 g from a priced spool + 50 g at the fallback 0.15 kr/g + 2 h.
        let cost = cost(&[(0, 100.0), (1, 50.0)], &prices, Some(2.0), &c, false).unwrap();
        assert_eq!((cost.filament, cost.machine, cost.total), (Some(20.5), Some(3.0), 23.5));
        // No prices anywhere: no cost.
        assert_eq!(super::cost(&[(1, 50.0)], &HashMap::new(), Some(2.0), &CostSettings::default(), false), None);
        // A filament without any price: filament unknown, machine still counts.
        let partial = super::cost(&[(1, 50.0)], &HashMap::new(), Some(2.0), &CostSettings { per_hour: Some(1.5), ..Default::default() }, false).unwrap();
        assert_eq!((partial.filament, partial.total), (None, 3.0));
        assert_eq!(use_per_tray(&[crate::models::SlicedFilament { id: 2, kind: "PLA".into(), color: String::new(), used_g: 10.0, used_m: 1.0 }], &[-1, 3], 0.5), vec![(3, 5.0)]);
    }
}
