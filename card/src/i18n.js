// The English text is the key; plurals are separate keys, as in the app.
const SV = {
  "Print history": "Tidigare utskrifter",
  "Search prints": "Sök utskrifter",
  "Loading…": "Hämtar…",
  "No prints yet.": "Inga utskrifter ännu.",
  "No prints match.": "Inga utskrifter matchar.",
  "Couldn't read the history: {0}": "Kunde inte läsa historiken: {0}",
  "Choose an AirSlice print sensor in the card's settings.": "Välj en utskriftssensor från AirSlice i kortets inställningar.",
  "{0} isn't available. Is the AirSlice integration set up?": "{0} är inte tillgänglig. Är integrationen AirSlice inställd?",
  "Show more": "Visa fler",
  "1 print": "1 utskrift",
  "{0} prints": "{0} utskrifter",
  Finished: "Klar",
  Failed: "Misslyckades",
  Stopped: "Stoppad",
  "Unknown result": "Okänt resultat",
  "Stopped at {0} %": "Stoppad vid {0} %",
  "Layer {0} of {1}": "Lager {0} av {1}",
  "Started outside AirSlice": "Startad utanför AirSlice",
  Started: "Startade",
  Ended: "Slutade",
  Filament: "Filament",
  "Machine and power": "Maskin och el",
  Total: "Totalt",
  "Estimated cost": "Beräknad kostnad",
  "Purge and prime tower": "Utspolning och prime tower",
  "1 filament change": "1 filamentbyte",
  "{0} filament changes": "{0} filamentbyten",
  Energy: "Energi",
  "Open the model page": "Öppna modellsidan",
  "Show details": "Visa detaljer",
  // The card's settings
  "Print sensor": "Utskriftssensor",
  Title: "Rubrik",
  "Prints shown at first": "Utskrifter som visas först",
  "Show the totals": "Visa summorna",
  "The sensor AirSlice has for each printer.": "Sensorn som AirSlice har för varje skrivare.",
};

export function langOf(hass) {
  return String(hass?.locale?.language || hass?.language || "en").toLowerCase().startsWith("sv") ? "sv" : "en";
}

export function t(lang, en, args = []) {
  const text = lang === "sv" ? (SV[en] ?? en) : en;
  return text.replace(/\{(\d+)\}/g, (_, i) => String(args[Number(i)] ?? ""));
}
