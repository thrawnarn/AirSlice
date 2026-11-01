import { useState } from "react";
import { api } from "./api";
import { tr } from "./i18n";
import type { AmsUnit, Printer, Tray } from "./types";
import { fmtMinutes, Notice, Sheet, toast } from "./ui";

const MIN_TEMP = 45;
const DEFAULT_HOURS = 8;
const NEW_ISSUE = "https://github.com/thrawnarn/AirSlice/issues/new";

/** Typical drying temperatures for spools whose tag says nothing. Longest prefix wins. */
const TYPICAL_TEMP: [string, number][] = [
  ["PLA", 50],
  ["PETG", 65],
  ["PET", 65],
  ["TPU", 65],
  ["PVA", 65],
  ["BVOH", 65],
  ["ABS", 80],
  ["ASA", 80],
  ["HIPS", 80],
  ["PA", 80],
  ["PC", 80],
];

function spoolTemp(t: Tray): number {
  if (t.dry_temp) return t.dry_temp;
  const kind = t.type.toUpperCase();
  const match = TYPICAL_TEMP.filter(([k]) => kind.startsWith(k)).sort((a, b) => b[0].length - a[0].length)[0];
  return match ? match[1] : MIN_TEMP;
}

/** The lowest temperature any loaded spool wants, so none of them softens, and the longest time. */
export function suggestDrying(trays: Tray[], maxTemp: number): { temp: number; hours: number } {
  const loaded = trays.filter((t) => t.type);
  const temp = loaded.length ? Math.min(...loaded.map(spoolTemp)) : MIN_TEMP;
  const hours = Math.max(0, ...loaded.map((t) => t.dry_hours ?? 0)) || DEFAULT_HOURS;
  return { temp: Math.min(Math.max(temp, MIN_TEMP), maxTemp), hours: Math.min(hours, 24) };
}

function DrySheet({ printer, unit, trays, onClose }: { printer: Printer; unit: AmsUnit; trays: Tray[]; onClose: () => void }) {
  const max = unit.dry_max_temp!;
  const suggested = suggestDrying(trays, max);
  const [temp, setTemp] = useState(String(suggested.temp));
  const [hours, setHours] = useState(String(suggested.hours));
  const [rotate, setRotate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const t = Number(temp);
  const h = Number(hours);
  const valid = Number.isInteger(t) && t >= MIN_TEMP && t <= max && Number.isInteger(h) && h >= 1 && h <= 24;

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const r = await api.post<{ started: boolean }>(`printers/${printer.id}/ams/${unit.id}/dry`, { temp: t, hours: h, rotate });
      toast(r.started ? tr("{0} is drying", [unit.name]) : tr("Sent, but {0} hasn't confirmed yet. Check the printer screen.", [printer.name]));
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Sheet title={tr("Dry {0}", [unit.name])} onClose={onClose}>
      <p className="muted small" style={{ marginTop: 0 }}>
        {tr("Suggested from the loaded spools: the lowest temperature any of them wants, so none of them softens.")}
      </p>
      <label className="field">
        <span>{tr("Temperature (°C)")}</span>
        <input className="input" type="number" inputMode="numeric" min={MIN_TEMP} max={max} value={temp} onChange={(e) => setTemp(e.target.value)} />
        <small>{tr("{0} to {1} °C on this unit", [MIN_TEMP, max])}</small>
      </label>
      <label className="field">
        <span>{tr("Hours")}</span>
        <input className="input" type="number" inputMode="numeric" min={1} max={24} value={hours} onChange={(e) => setHours(e.target.value)} />
        <small>{tr("1 to 24 hours")}</small>
      </label>
      <label className="row" style={{ marginBottom: 14 }}>
        <input type="checkbox" checked={rotate} onChange={(e) => setRotate(e.target.checked)} />
        <span>{tr("Turn the spools while drying")}</span>
      </label>
      {t > suggested.temp && <Notice tone="warn">{tr("This is hotter than the suggested {0} °C and can soften the filament on the spools.", [suggested.temp])}</Notice>}
      {error && <Notice tone="err">{error}</Notice>}
      <button className="btn primary block" type="button" disabled={!valid || busy} onClick={start}>
        {busy ? tr("Starting…") : tr("Start drying")}
      </button>
    </Sheet>
  );
}

/** Starting drying is only known to fail on some models, and not yet known to work on any. */
function UnconfirmedSheet({ printer, onContinue, onClose }: { printer: Printer; onContinue: () => void; onClose: () => void }) {
  return (
    <Sheet title={tr("Drying isn't confirmed on this printer")} onClose={onClose}>
      <Notice tone="warn">{tr("Starting drying from AirSlice hasn't been confirmed on {0}. The P1S and P1P accept the command and then ignore it, so it may do nothing here too.", [printer.printer_model])}</Notice>
      <p className="small">
        {tr("Please tell us whether it worked, so others know.")}{" "}
        <a href={`${NEW_ISSUE}?title=${encodeURIComponent(`AMS drying on ${printer.printer_model}`)}`} target="_blank" rel="noreferrer">
          {tr("Report it on GitHub")}
        </a>
      </p>
      <button className="btn primary block" type="button" onClick={onContinue}>
        {tr("Continue")}
      </button>
    </Sheet>
  );
}

/** Drying state and controls in an AMS unit's heading; nothing for a unit that can't dry. */
export function DryingControl({ printer, unit, trays }: { printer: Printer; unit: AmsUnit; trays: Tray[] }) {
  const [step, setStep] = useState<"notice" | "settings" | null>(null);
  const [busy, setBusy] = useState(false);
  const controls = printer.can_start_drying !== false;

  async function stop() {
    if (!window.confirm(tr("Stop drying {0}?", [unit.name]))) return;
    setBusy(true);
    try {
      const r = await api.del<{ stopped: boolean }>(`printers/${printer.id}/ams/${unit.id}/dry`);
      toast(r.stopped ? tr("Stopped drying {0}", [unit.name]) : tr("Sent, but {0} hasn't confirmed yet. Check the printer screen.", [printer.name]));
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (unit.dry_minutes) {
    return (
      <>
        <span className="pc-drying">{tr("Drying, {0} left", [fmtMinutes(unit.dry_minutes)])}</span>
        {controls && (
          <button type="button" className="linkish small" disabled={busy} onClick={stop} aria-label={tr("Stop drying {0}", [unit.name])}>
            {tr("Stop")}
          </button>
        )}
      </>
    );
  }
  if (!controls || !unit.dry_max_temp) return null;
  if (unit.in_print) return <span className="faint">{tr("Can't dry during this print")}</span>;
  return (
    <>
      <button type="button" className="linkish small" onClick={() => setStep("notice")} aria-label={tr("Dry {0}", [unit.name])}>
        {tr("Dry")}
      </button>
      {step === "notice" && <UnconfirmedSheet printer={printer} onContinue={() => setStep("settings")} onClose={() => setStep(null)} />}
      {step === "settings" && <DrySheet printer={printer} unit={unit} trays={trays} onClose={() => setStep(null)} />}
    </>
  );
}
