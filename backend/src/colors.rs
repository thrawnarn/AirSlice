//! Colour names and matching a colour someone asks for ("red", "white PETG",
//! "#1E90FF") to a loaded tray. Used by the MCP server.

use crate::printer::status::Tray;

/// Name, `RRGGBB`; a name can have several shades. Output names are these;
/// the order breaks ties.
const NAMES: &[(&str, &str)] = &[
    ("white", "FFFFFF"),
    ("black", "000000"),
    ("grey", "8E9089"),
    ("dark grey", "4D4D4D"),
    ("light grey", "D1D3D5"),
    ("silver", "A6A9AA"),
    ("red", "C12E1F"),
    ("dark red", "7D1A1A"),
    ("orange", "FF6A13"),
    ("yellow", "F4EE2A"),
    ("gold", "D4AF37"),
    ("beige", "E8D8B4"),
    ("brown", "7B4A2B"),
    ("green", "00AE42"),
    ("dark green", "1F5132"),
    ("lime", "A6E22E"),
    ("teal", "008080"),
    ("cyan", "0086D6"),
    ("light blue", "8DC7F0"),
    ("blue", "0A2CA5"),
    ("navy", "1B2A4A"),
    ("purple", "5E43B7"),
    ("magenta", "EC008C"),
    ("pink", "F5A3C7"),
    // Pure and web shades, so #FF0000 is red and not orange.
    ("red", "FF0000"),
    ("red", "E30000"),
    ("green", "00FF00"),
    ("green", "008000"),
    ("blue", "0000FF"),
    ("blue", "1E50D2"),
    ("yellow", "FFFF00"),
    ("yellow", "FFD100"),
    ("orange", "FFA500"),
    ("cyan", "00FFFF"),
    ("magenta", "FF00FF"),
    ("purple", "800080"),
    ("pink", "FFC0CB"),
    ("brown", "8B4513"),
    ("grey", "808080"),
];

/// Other words for the names above, Swedish included.
const ALIASES: &[(&str, &str)] = &[
    ("gray", "grey"),
    ("dark gray", "dark grey"),
    ("light gray", "light grey"),
    ("vit", "white"),
    ("svart", "black"),
    ("grå", "grey"),
    ("mörkgrå", "dark grey"),
    ("ljusgrå", "light grey"),
    ("röd", "red"),
    ("mörkröd", "dark red"),
    ("gul", "yellow"),
    ("guld", "gold"),
    ("brun", "brown"),
    ("grön", "green"),
    ("mörkgrön", "dark green"),
    ("turkos", "teal"),
    ("ljusblå", "light blue"),
    ("blå", "blue"),
    ("marinblå", "navy"),
    ("lila", "purple"),
    ("rosa", "pink"),
    ("violet", "purple"),
    ("transparent", "white"),
    ("clear", "white"),
];

/// How far (CIE76 ΔE) a tray may be from a colour asked by name or code and
/// still count as that colour. Red vs orange is ~50, white vs light grey ~17.
const MAX_DISTANCE: f64 = 35.0;

fn rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() < 6 || !h.is_char_boundary(6) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

fn lab([r, g, b]: [u8; 3]) -> [f64; 3] {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c > 0.04045 { ((c + 0.055) / 1.055).powf(2.4) } else { c / 12.92 }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (lab(a), lab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The nearest plain name for a `#RRGGBB` colour.
pub fn name(hex: &str) -> &'static str {
    let Some(c) = rgb(hex) else { return "unknown" };
    NAMES
        .iter()
        .map(|(n, h)| (*n, distance(c, rgb(h).unwrap())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| n)
        .unwrap_or("unknown")
}

fn named(word: &str) -> Option<&'static str> {
    let w = word.trim();
    NAMES
        .iter()
        .map(|(n, _)| *n)
        .find(|n| n.eq_ignore_ascii_case(w))
        .or_else(|| ALIASES.iter().find(|(a, _)| a.eq_ignore_ascii_case(w) || a.to_lowercase() == w.to_lowercase()).map(|(_, n)| *n))
}

/// Where a tray sits, for people: "AMS 1 slot 2", "AMS HT", "external spool".
pub fn place(t: &Tray) -> String {
    match (t.ams, t.slot) {
        _ if t.id == crate::printer::status::EXTERNAL_TRAY => "external spool".into(),
        (Some(a), _) if a >= 128 => format!("AMS HT {}", a - 127),
        (Some(a), Some(s)) => format!("AMS {} slot {}", a + 1, s + 1),
        _ => format!("tray {}", t.id),
    }
}

/// Pick the loaded tray for one asked colour. `ask` is a tray id, a colour
/// name or `#RRGGBB`, optionally with a material ("red PETG").
pub fn pick<'a>(ask: &serde_json::Value, trays: &[&'a Tray]) -> Result<&'a Tray, String> {
    if let Some(id) = ask.as_i64() {
        return trays.iter().find(|t| t.id as i64 == id).copied().ok_or_else(|| format!("tray {id} is empty or not reported by the printer"));
    }
    let text = ask.as_str().ok_or("a colour must be a name, a #RRGGBB code or a tray id")?.trim();
    if let Some(id) = text.strip_prefix("tray ").and_then(|n| n.trim().parse::<i64>().ok()) {
        return pick(&serde_json::json!(id), trays);
    }
    // A material word that some loaded tray has, e.g. "PETG" or "PLA-CF".
    let lower = text.to_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    let material = words
        .iter()
        .position(|w| trays.iter().any(|t| t.kind.eq_ignore_ascii_case(w)))
        .map(|i| words.remove(i).to_string());
    let rest = words.join(" ");
    let candidates: Vec<&Tray> = trays.iter().copied().filter(|t| material.as_deref().is_none_or(|m| t.kind.eq_ignore_ascii_case(m))).collect();
    if candidates.is_empty() {
        return Err(format!("no {} is loaded", material.unwrap_or_default().to_uppercase()));
    }
    if rest.is_empty() {
        return match candidates.as_slice() {
            [one] => Ok(one),
            _ => Err(format!("several trays have {}: say which colour", text.to_uppercase())),
        };
    }
    let (targets, by_name): (Vec<[u8; 3]>, _) = match rgb(&rest).filter(|_| rest.trim_start_matches('#').len() == 6) {
        Some(c) => (vec![c], None),
        None => {
            let n = named(&rest).ok_or_else(|| format!("unknown colour {rest:?}: use a colour name or a #RRGGBB code"))?;
            (NAMES.iter().filter(|(k, _)| *k == n).filter_map(|(_, h)| rgb(h)).collect(), Some(n))
        }
    };
    let near = |t: &&Tray| rgb(&t.color).map(|c| targets.iter().map(|x| distance(c, *x)).fold(f64::MAX, f64::min)).unwrap_or(f64::MAX);
    // Trays whose own nearest name is the one asked for come first.
    let best = candidates
        .iter()
        .filter(|t| by_name.is_some_and(|n| name(&t.color) == n))
        .min_by(|a, b| near(a).total_cmp(&near(b)))
        .or_else(|| candidates.iter().filter(|t| near(t) <= MAX_DISTANCE).min_by(|a, b| near(a).total_cmp(&near(b))));
    best.copied().ok_or_else(|| format!("nothing loaded looks {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tray(id: i32, kind: &str, color: &str) -> Tray {
        Tray { id, ams: Some(0), slot: Some(id as u32), kind: kind.into(), color: color.into(), ..Default::default() }
    }

    #[test]
    fn names_colours() {
        assert_eq!(name("#FFFFFF"), "white");
        assert_eq!(name("#000000"), "black");
        assert_eq!(name("#C12E1F"), "red");
        assert_eq!(name("#E30000"), "red");
        assert_eq!(name("#FF0000"), "red");
        assert_eq!(name("#FF6A13"), "orange");
        assert_eq!(name("#FFA500"), "orange");
        assert_eq!(name("#0000FF"), "blue");
        assert_eq!(name("#00AE42"), "green");
        assert_eq!(name("#0A2CA5"), "blue");
        assert_eq!(name("bad"), "unknown");
    }

    #[test]
    fn picks_trays() {
        let t = [tray(0, "PLA", "#FFFFFF"), tray(1, "PLA", "#161616"), tray(2, "PETG", "#C12E1F"), tray(3, "PLA", "#B02020")];
        let trays: Vec<&Tray> = t.iter().collect();
        let id = |v: serde_json::Value| pick(&v, &trays).map(|t| t.id);
        assert_eq!(id(json!("white")), Ok(0));
        assert_eq!(id(json!("Svart")), Ok(1));
        assert_eq!(id(json!("red")), Ok(2));
        assert_eq!(id(json!("red PLA")), Ok(3));
        assert_eq!(id(json!("röd petg")), Ok(2));
        assert_eq!(id(json!("PETG")), Ok(2));
        assert_eq!(id(json!("#FAFAFA")), Ok(0));
        assert_eq!(id(json!(3)), Ok(3));
        assert_eq!(id(json!("tray 1")), Ok(1));
        assert!(id(json!("blue")).is_err());
        assert!(id(json!("PLA")).is_err(), "three PLA trays");
        assert!(id(json!("white ABS")).is_err(), "ABS isn't a loaded material, so it's an unknown colour word");
        assert!(id(json!(7)).is_err());
        assert!(id(json!("chartreuse")).is_err());
    }
}
