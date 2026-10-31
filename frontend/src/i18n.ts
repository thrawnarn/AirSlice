// Translations: the English text is the key, so untranslated text simply
// shows in English. Placeholders are {0}, {1}… (or {name}).
import { sv } from "./sv";

export type Lang = "en" | "sv";
export type LangChoice = Lang | "auto";

const KEY = "airslice-lang";

export function langChoice(): LangChoice {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "en" || v === "sv") return v;
  } catch {
    /* private mode */
  }
  return "auto";
}

function detect(): Lang {
  const choice = langChoice();
  if (choice !== "auto") return choice;
  const wanted = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const l of wanted) {
    const code = l.toLowerCase();
    if (code.startsWith("sv")) return "sv";
    if (code.startsWith("en")) return "en";
  }
  return "en";
}

export const lang: Lang = detect();
if (typeof document !== "undefined") document.documentElement.lang = lang;

/** Save the choice and reload, so every text is in the new language. */
export function setLangChoice(choice: LangChoice) {
  try {
    if (choice === "auto") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    /* private mode: only this visit */
  }
  window.location.reload();
}

const table: Record<string, string> = lang === "sv" ? sv : {};

/** The text in the chosen language, with `{0}`, `{1}`… or `{name}` filled in. */
export function tr(text: string, vars?: (string | number | null | undefined)[] | Record<string, string | number | null | undefined>): string {
  let out = table[text] ?? text;
  if (vars) {
    out = out.replace(/\{(\w+)\}/g, (m, k: string) => {
      const v = Array.isArray(vars) ? vars[Number(k)] : vars[k];
      return v == null ? m : String(v);
    });
  }
  return out;
}

/** Locale for numbers and dates. */
export const locale = lang === "sv" ? "sv-SE" : undefined;
