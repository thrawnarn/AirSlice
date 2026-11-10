//! Translations for what AirSlice sends out (notifications). As in the
//! frontend, the English text is the key and {0}, {1}… are placeholders;
//! text without a Swedish entry goes out in English.

use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Sv,
}

impl Lang {
    /// "sv", "sv-SE" → Swedish; anything else English.
    pub fn from_code(code: &str) -> Lang {
        if code.trim().to_lowercase().starts_with("sv") { Lang::Sv } else { Lang::En }
    }
}

const SV: &[(&str, &str)] = &[
    ("Sliced, ready to print", "Slicad, klar att skriva ut"),
    ("{0} — {1}, {2}", "{0} — {1}, {2}"),
    ("Printing on {0}", "Skriver ut på {0}"),
    ("About {0}, ready around {1}", "Cirka {0}, klar runt {1}"),
    ("Estimated cost {0}, {1}", "Beräknad kostnad {0}, {1}"),
    ("Estimated cost {0}", "Beräknad kostnad {0}"),
    ("First layer done on {0}", "Första lagret klart på {0}"),
    ("{0} — check that it sticks", "{0} — kolla att det fastnar"),
    ("Printing for {0}, ready around {1}", "Har skrivit ut i {0}, klar runt {1}"),
    ("Printing for {0}", "Har skrivit ut i {0}"),
    ("Printed on {0}", "Utskriven på {0}"),
    ("Print stopped on {0}", "Utskriften stoppad på {0}"),
    ("Print failed on {0}", "Utskriften misslyckades på {0}"),
    ("Took {0} (estimated {1}), done {2}", "Tog {0} (beräknat {1}), klar {2}"),
    ("Took {0}, done {1}", "Tog {0}, klar {1}"),
    ("Cost {0}", "Kostnad {0}"),
    ("Cost {0}, {1} kWh", "Kostnad {0}, {1} kWh"),
    ("Used {0} kWh", "Använde {0} kWh"),
    ("{0} is free — 1 print waiting", "{0} är ledig — 1 utskrift väntar"),
    ("{0} is free — {1} prints waiting", "{0} är ledig — {1} utskrifter väntar"),
    ("Next: {0}, {1}", "Näst på tur: {0}, {1}"),
    ("Next: {0}", "Näst på tur: {0}"),
    ("{0} in the queue", "{0} i kön"),
    ("Clear the plate, then start it.", "Töm plattan och starta den sedan."),
    ("Problem with {0}", "Problem med {0}"),
    ("Something went wrong", "Något gick fel"),
    ("{0} has paused", "{0} har pausat"),
    ("{0} needs attention", "{0} behöver tillsyn"),
    ("Paused at {0} %, layer {1} of {2}", "Pausad vid {0} %, lager {1} av {2}"),
    ("Paused at {0} %", "Pausad vid {0} %"),
    ("The printer reports error {0}.", "Skrivaren rapporterar fel {0}."),
    ("The printer doesn't say why. It may have been paused on its screen.", "Skrivaren säger inte varför. Den kan ha pausats på sin skärm."),
    ("Lost contact with {0}", "Tappade kontakten med {0}"),
    ("The print may still be running. Check the printer.", "Utskriften kan fortfarande pågå. Titta till skrivaren."),
    ("{0} is back", "{0} är tillbaka"),
    ("tomorrow {0}", "i morgon {0}"),
    ("Notifications work.", "Notiserna fungerar."),
];

const SV_DAYS: [&str; 7] = ["mån", "tis", "ons", "tors", "fre", "lör", "sön"];

/// Translate `en` and fill in the placeholders.
pub fn t(lang: Lang, en: &str, args: &[&dyn Display]) -> String {
    let text = match lang {
        Lang::Sv => SV.iter().find(|(k, _)| *k == en).map(|(_, v)| *v).unwrap_or(en),
        Lang::En => en,
    };
    let mut out = text.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out
}

/// "14:05", "tomorrow 07:30", a weekday within the week, else the date.
pub fn clock(lang: Lang, at: &jiff::Zoned, today: &jiff::Zoned) -> String {
    let hm = at.strftime("%H:%M").to_string();
    match (at.date() - today.date()).get_days() {
        0 => hm,
        1 => t(lang, "tomorrow {0}", &[&hm]),
        2..=6 => {
            let day = match lang {
                Lang::Sv => SV_DAYS[at.weekday().to_monday_zero_offset() as usize].to_string(),
                Lang::En => at.strftime("%a").to_string(),
            };
            format!("{day} {hm}")
        }
        _ => format!("{} {hm}", at.strftime("%Y-%m-%d")),
    }
}

/// "9.07 kr" / "9,07 kr".
pub fn money(lang: Lang, total: f64, currency: &str) -> String {
    let n = format!("{total:.2}");
    let n = if lang == Lang::Sv { n.replace('.', ",") } else { n };
    format!("{n} {currency}").trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_fills_in() {
        assert_eq!(t(Lang::Sv, "Printing on {0}", &[&"Workshop"]), "Skriver ut på Workshop");
        assert_eq!(t(Lang::En, "Printing on {0}", &[&"Workshop"]), "Printing on Workshop");
        assert_eq!(t(Lang::Sv, "Not in the table {0}", &[&1]), "Not in the table 1");
        assert_eq!(money(Lang::Sv, 9.071, "kr"), "9,07 kr");
        assert_eq!(Lang::from_code("sv-SE"), Lang::Sv);
    }

    /// Every text the notifications use has a Swedish entry.
    #[test]
    fn notification_texts_are_translated() {
        let re = regex::Regex::new(r#"t\(l(?:ang)?, "((?:[^"\\]|\\.)*)""#).unwrap();
        let mut keys: Vec<String> = Vec::new();
        for src in [include_str!("events.rs"), include_str!("notify.rs"), include_str!("api/settings.rs")] {
            keys.extend(re.captures_iter(src).map(|c| c[1].to_string()));
        }
        // Texts picked from a list, then passed to `t` as a variable.
        keys.extend(["Printed on {0}", "Print stopped on {0}", "Print failed on {0}"].map(String::from));
        assert!(keys.len() > 15);
        let missing: Vec<&String> = keys.iter().filter(|k| !SV.iter().any(|(e, _)| e == k)).collect();
        assert!(missing.is_empty(), "no Swedish for {missing:?}");
    }

    #[test]
    fn clock_names_the_day() {
        let tz = jiff::tz::TimeZone::get("Europe/Stockholm").unwrap();
        let today: jiff::Zoned = "2026-10-30T09:00[Europe/Stockholm]".parse().unwrap();
        let later = today.checked_add(jiff::Span::new().hours(3)).unwrap();
        let tomorrow = today.checked_add(jiff::Span::new().hours(22)).unwrap();
        let monday = jiff::civil::date(2026, 11, 2).at(8, 0, 0, 0).to_zoned(tz).unwrap();
        assert_eq!(clock(Lang::Sv, &later, &today), "12:00");
        assert_eq!(clock(Lang::Sv, &tomorrow, &today), "i morgon 07:00");
        assert_eq!(clock(Lang::En, &tomorrow, &today), "tomorrow 07:00");
        assert_eq!(clock(Lang::Sv, &monday, &today), "mån 08:00");
    }
}
