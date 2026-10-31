import { useEffect, useState } from "react";
import { api } from "./api";
import type { SlicePreset, SliceTweaks } from "./types";
import { toast } from "./ui";
import { tr } from "./i18n";

const LAYERS = [0.08, 0.12, 0.16, 0.2, 0.24, 0.28];
const INFILL = [0, 5, 10, 15, 20, 25, 30, 40, 50, 60, 80, 100];

/** Empty fields keep the profile's or file's own setting. */
export function cleanTweaks(t: SliceTweaks): SliceTweaks | undefined {
  const out: SliceTweaks = {};
  if (t.layer_height != null) out.layer_height = t.layer_height;
  if (t.walls != null) out.walls = t.walls;
  if (t.infill != null) out.infill = t.infill;
  if (t.supports) out.supports = t.supports;
  if (t.brim != null) out.brim = t.brim;
  return Object.keys(out).length ? out : undefined;
}

export function describeTweaks(t: SliceTweaks): string {
  const parts: string[] = [];
  if (t.layer_height != null) parts.push(tr("{0} mm layers", [t.layer_height]));
  if (t.walls != null) parts.push(`${t.walls} walls`);
  if (t.infill != null) parts.push(tr("{0} % infill", [t.infill]));
  if (t.supports) parts.push(t.supports === "off" ? tr("no supports") : `${t.supports} supports`);
  if (t.brim != null) parts.push(t.brim ? "brim" : tr("no brim"));
  return parts.join(", ");
}

const same = (a: SliceTweaks, b: SliceTweaks) => JSON.stringify(cleanTweaks(a) ?? {}) === JSON.stringify(cleanTweaks(b) ?? {});

/** Saved presets plus quick changes to layer height, walls, infill, supports and brim. */
export function TweakPicker({ value, onChange }: { value: SliceTweaks; onChange: (t: SliceTweaks) => void }) {
  const [presets, setPresets] = useState<SlicePreset[]>([]);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    api
      .get<SlicePreset[]>("settings/presets")
      .then(setPresets)
      .catch(() => setPresets([]));
  }, []);

  async function savePresets(list: SlicePreset[], done: string) {
    try {
      setPresets(await api.put<SlicePreset[]>("settings/presets", list));
      toast(done);
    } catch (e) {
      toast((e as Error).message);
    }
  }

  function saveAs() {
    const t = cleanTweaks(value);
    if (!t) return;
    const name = window.prompt(tr("Name for these settings"), "")?.trim();
    if (!name) return;
    const rest = presets.filter((p) => p.name.toLowerCase() !== name.toLowerCase());
    void savePresets([...rest, { name, ...t }], tr("Saved “{0}”", [name]));
  }

  const set = (patch: Partial<SliceTweaks>) => onChange({ ...value, ...patch });
  const chosen = presets.find((p) => same(p, value));
  const changed = cleanTweaks(value);
  return (
    <div className="field tweaks">
      <span>{tr("Quick settings")}</span>
      <div className="chips" role="group" aria-label={tr("Saved settings")}>
        <button type="button" className="chip" aria-pressed={!changed} onClick={() => onChange({})}>
          {tr("As set up")}
        </button>
        {presets.map((p) => (
          <button key={p.name} type="button" className="chip" aria-pressed={chosen?.name === p.name} onClick={() => onChange(cleanTweaks(p) ?? {})} title={describeTweaks(p)}>
            {tr(p.name)}
          </button>
        ))}
      </div>
      {changed && <small>{tr("Changed: {0}", [describeTweaks(value)])}</small>}
      <details open={open} onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)} style={{ marginTop: 8 }}>
        <summary className="small muted">{tr("Adjust layers, walls, infill, supports and brim")}</summary>
        <div className="tweak-grid">
          <label className="field">
            <span>{tr("Layer height")}</span>
            <select className="input" value={value.layer_height ?? ""} onChange={(e) => set({ layer_height: e.target.value ? Number(e.target.value) : undefined })}>
              <option value="">{tr("As set up")}</option>
              {LAYERS.map((h) => (
                <option key={h} value={h}>
                  {h} mm{h <= 0.12 ? tr(" (fine)") : h >= 0.28 ? tr(" (fast)") : ""}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>{tr("Walls")}</span>
            <select className="input" value={value.walls ?? ""} onChange={(e) => set({ walls: e.target.value ? Number(e.target.value) : undefined })}>
              <option value="">{tr("As set up")}</option>
              {[1, 2, 3, 4, 5, 6, 8].map((w) => (
                <option key={w} value={w}>
                  {w}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>{tr("Infill")}</span>
            <select className="input" value={value.infill ?? ""} onChange={(e) => set({ infill: e.target.value ? Number(e.target.value) : undefined })}>
              <option value="">{tr("As set up")}</option>
              {INFILL.map((i) => (
                <option key={i} value={i}>
                  {i} %
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>{tr("Supports")}</span>
            <select className="input" value={value.supports ?? ""} onChange={(e) => set({ supports: (e.target.value || undefined) as SliceTweaks["supports"] })}>
              <option value="">{tr("As set up")}</option>
              <option value="off">{tr("None")}</option>
              <option value="normal">{tr("Normal")}</option>
              <option value="tree">{tr("Tree")}</option>
            </select>
          </label>
          <label className="field">
            <span>{tr("Brim")}</span>
            <select className="input" value={value.brim == null ? "" : value.brim ? "on" : "off"} onChange={(e) => set({ brim: e.target.value === "" ? undefined : e.target.value === "on" })}>
              <option value="">{tr("As set up")}</option>
              <option value="on">{tr("Yes (5 mm)")}</option>
              <option value="off">{tr("No")}</option>
            </select>
          </label>
        </div>
        <div className="row wrap" style={{ gap: 8 }}>
          <button type="button" className="btn small" onClick={saveAs} disabled={!changed}>
            {tr("Save as preset…")}
          </button>
          {chosen && (
            <button
              type="button"
              className="btn small ghost"
              onClick={() => {
                if (window.confirm(tr("Delete the preset “{0}”?", [chosen.name]))) void savePresets(presets.filter((p) => p.name !== chosen.name), tr("Deleted “{0}”", [chosen.name]));
              }}
            >
              {tr("Delete “{0}”", [chosen.name])}
            </button>
          )}
        </div>
      </details>
    </div>
  );
}
