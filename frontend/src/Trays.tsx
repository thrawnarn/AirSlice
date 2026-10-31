import { useEffect, useMemo, useState } from "react";
import { api } from "./api";
import type { BuildPlate, NamedProfile, Printer, ProfileChoices, Spool as SpoolmanSpool, Tray } from "./types";
import { Notice, Sheet, Spool, toast, trayLabel } from "./ui";
import { locale, tr } from "./i18n";

/** Every slot the printer reports, empty ones included, then the external spool. */
export function allTrays(printer: Printer): Tray[] {
  const s = printer.status;
  return [...s.trays, ...(s.external ? [s.external] : [])];
}

/** Trays with filament in them, the ones a print can use. */
export function loadedTrays(printer: Printer | null): Tray[] {
  return printer ? allTrays(printer).filter((t) => t.type && t.color) : [];
}

export type SpoolInfo = { configured: boolean; trays?: Record<string, SpoolmanSpool>; spools?: SpoolmanSpool[] };

/** Spoolman spools for a printer's trays; `null` while loading or when off. */
export function usePrinterSpools(printer: Printer | null): [SpoolInfo | null, () => void] {
  const [info, setInfo] = useState<SpoolInfo | null>(null);
  const [n, setN] = useState(0);
  const trayKey = printer ? allTrays(printer).map((t) => `${t.id}${t.type}${t.color}`).join() : "";
  useEffect(() => {
    if (!printer) return;
    api
      .get<SpoolInfo>(`printers/${printer.id}/spools`)
      .then(setInfo)
      .catch(() => setInfo(null));
  }, [printer?.id, trayKey, n]);
  return [info, () => setN((x) => x + 1)];
}

export function spoolLine(s: SpoolmanSpool | undefined | null): string {
  if (!s) return "";
  return `Spoolman #${s.id}${s.remaining_g != null ? `, ${Math.round(s.remaining_g)} g left` : ""}`;
}

export function money(c: { total: number; currency: string }): string {
  const v = c.total >= 100 ? Math.round(c.total).toLocaleString(locale) : c.total.toLocaleString(locale, { minimumFractionDigits: Number.isInteger(c.total) ? 0 : 2, maximumFractionDigits: 2 });
  return c.currency ? `${v} ${c.currency}` : v;
}

let platesRequest: Promise<BuildPlate[]> | null = null;

/** Build plates the slicer knows (fetched once). */
export function usePlates(): BuildPlate[] {
  const [plates, setPlates] = useState<BuildPlate[]>([]);
  useEffect(() => {
    platesRequest ??= api.get<BuildPlate[]>("plates");
    platesRequest.then(setPlates).catch(() => {
      platesRequest = null;
    });
  }, []);
  return plates;
}

export function plateName(plates: BuildPlate[], value: string | null | undefined): string {
  if (!value) return "";
  return plates.find((p) => p.value.toLowerCase() === value.toLowerCase())?.label ?? value;
}

// Common Bambu spool colours, then a custom one.
const PALETTE = ["#FFFFFF", "#000000", "#8E9089", "#C12E1F", "#F4A925", "#FEC600", "#00AE42", "#0A2989", "#5E43B7", "#F55A74", "#9D432C", "#E8DBB7"];

/**
 * What is loaded in each tray, with ways to put it right: refresh from the
 * printer, re-read a spool's RFID tag, or set material and colour by hand.
 */
export function TraysSheet({ printer, onClose }: { printer: Printer; onClose: () => void }) {
  const [editing, setEditing] = useState<number | null>(null);
  const [spools, reloadSpools] = usePrinterSpools(printer);
  const [pending, setPending] = useState<string>("");
  const trays = allTrays(printer);
  const edited = trays.find((t) => t.id === editing) ?? null;

  async function act(key: string, fn: () => Promise<unknown>, done: string) {
    setPending(key);
    try {
      await fn();
      toast(done);
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setPending("");
    }
  }
  const refresh = () => act("refresh", () => api.post(`printers/${printer.id}/control`, { action: "refresh" }), tr("Asked the printer for its trays"));
  const reread = (t: Tray) =>
    act(`reread${t.id}`, () => api.post(`printers/${printer.id}/trays/${t.id}/reread`), tr("Reading the spool in {0} — takes a few seconds", [trayLabel(t)]));

  return (
    <Sheet title={edited ? `${trayLabel(edited)}` : tr("Trays on {0}", [printer.name])} onClose={onClose}>
      {edited ? (
        <TrayEditor
          printer={printer}
          tray={edited}
          spools={spools?.configured ? spools : null}
          onDone={() => {
            setEditing(null);
            reloadSpools();
          }}
        />
      ) : (
        <>
          {!printer.status.connected && <Notice tone="warn">{tr("{0} is offline, so trays can't be changed right now.", [printer.name])}</Notice>}
          <ul className="tray-list">
            {trays.map((t) => (
              <li key={t.id} className="tray-row">
                <Spool color={t.color} empty={!t.type} size={36} label={`${trayLabel(t)}: ${t.type || tr("empty")}`} />
                <div className="grow" style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 620 }}>{trayLabel(t)}</div>
                  <div className="faint ellipsis">
                    {t.type ? `${t.name ?? t.type}${t.info_idx ? "" : ", no RFID"}` : tr("Empty")}
                    {t.remain != null && t.type ? `, ${t.remain}%` : ""}
                  </div>
                  {spools?.trays?.[t.id] && <div className="faint ellipsis small">{spoolLine(spools.trays[t.id])}</div>}
                </div>
                {t.id !== 254 && (
                  <button className="btn small ghost" disabled={!printer.status.connected || !!pending} onClick={() => reread(t)}>
                    {pending === `reread${t.id}` ? tr("Reading…") : tr("Re-read")}
                  </button>
                )}
                <button className="btn small" disabled={!printer.status.connected} onClick={() => setEditing(t.id)}>
                  {tr("Edit")}
                </button>
              </li>
            ))}
          </ul>
          <p className="faint small">
            {tr("Re-read asks the AMS to read the spool's RFID tag again. Edit sets material and colour for spools without a tag, or when the tag was read wrong.")}
          </p>
          <button className="btn block" disabled={!printer.status.connected || !!pending} onClick={refresh}>
            {pending === "refresh" ? tr("Refreshing…") : tr("Refresh from the printer")}
          </button>
        </>
      )}
    </Sheet>
  );
}

function TrayEditor({ printer, tray, spools, onDone }: { printer: Printer; tray: Tray; spools: SpoolInfo | null; onDone: () => void }) {
  const linked = spools?.trays?.[tray.id] ?? null;
  const [spoolId, setSpoolId] = useState<string>(linked ? String(linked.id) : "");
  const [choices, setChoices] = useState<NamedProfile[] | null>(null);
  const [profile, setProfile] = useState("");
  const [color, setColor] = useState(tray.color || "#FFFFFF");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .get<ProfileChoices>(`profiles?machine=${encodeURIComponent(printer.machine_profile)}`)
      .then((c) => {
        setChoices(c.filaments);
        setProfile(guessProfile(c.filaments, tray));
      })
      .catch((e) => setError((e as Error).message));
  }, [printer.machine_profile, tray.id]);

  // Bambu and generic profiles, grouped by material.
  const groups = useMemo(() => {
    const out = new Map<string, NamedProfile[]>();
    for (const f of choices ?? []) {
      if (!/^(Bambu|Generic) /.test(f.name) && f.name !== profile) continue;
      const k = f.filament_type ?? tr("Other");
      out.set(k, [...(out.get(k) ?? []), f]);
    }
    return [...out.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [choices, profile]);

  async function save() {
    setBusy(true);
    setError(null);
    try {
      if (spools && spoolId !== (linked ? String(linked.id) : "")) {
        await api.post(`printers/${printer.id}/trays/${tray.id}/spool`, { spool_id: spoolId ? Number(spoolId) : null });
      }
      await api.post(`printers/${printer.id}/trays/${tray.id}`, { profile, color });
      toast(tr("{0} set to {1}", [trayLabel(tray), profile.replace(/ @.*$/, "")]));
      onDone();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  return (
    <div>
      <div className="row" style={{ gap: 12, marginBottom: 14 }}>
        <Spool color={color} size={48} label={tr("New colour {0}", [color])} />
        <div className="faint">{tray.type ? tr("Now: {0}", [tray.name ?? tray.type]) : tr("Now: empty")}</div>
      </div>
      <label className="field">
        <span>{tr("Filament")}</span>
        <select className="input" value={profile} onChange={(e) => setProfile(e.target.value)} disabled={!choices}>
          {!choices && <option>{tr("Loading…")}</option>}
          {groups.map(([kind, list]) => (
            <optgroup key={kind} label={kind}>
              {list.map((f) => (
                <option key={f.name} value={f.name}>
                  {f.name.replace(/ @.*$/, "")}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>
      <div className="field">
        <span>{tr("Colour")}</span>
        <div className="swatches" role="group" aria-label={tr("Colour")} style={{ margin: "4px 0 0" }}>
          {PALETTE.map((c) => (
            <button
              key={c}
              type="button"
              className="swatch"
              style={{ background: c }}
              aria-pressed={color.toUpperCase() === c}
              aria-label={c}
              onClick={() => setColor(c)}
            />
          ))}
          <label className="swatch swatch-custom" title={tr("Another colour")}>
            <input type="color" value={color} onChange={(e) => setColor(e.target.value.toUpperCase())} aria-label={tr("Another colour")} />
          </label>
        </div>
      </div>
      {spools && (
        <label className="field">
          <span>{tr("Spoolman spool")}</span>
          <select
            className="input"
            value={spoolId}
            onChange={(e) => {
              setSpoolId(e.target.value);
              const sp = spools.spools?.find((x) => String(x.id) === e.target.value);
              if (sp?.color) setColor(sp.color);
            }}
          >
            <option value="">{tr("None")}</option>
            {spools.spools?.map((sp) => (
              <option key={sp.id} value={sp.id}>
                #{sp.id} {sp.name}
                {sp.remaining_g != null ? ` (${Math.round(sp.remaining_g)} g)` : ""}
              </option>
            ))}
          </select>
          <small>{tray.uuid ? tr("Spools with this RFID tag in Spoolman are found by themselves.") : tr("Which spool is in this tray, for what's left and its price.")}</small>
        </label>
      )}
      {error && <Notice tone="err">{error}</Notice>}
      <div className="row" style={{ gap: 8, marginTop: 16 }}>
        <button className="btn grow" onClick={onDone} disabled={busy}>
          {tr("Back")}
        </button>
        <button className="btn primary grow" onClick={save} disabled={busy || !profile}>
          {busy ? tr("Saving…") : tr("Save to printer")}
        </button>
      </div>
    </div>
  );
}

/** The spool's own profile when its RFID is known, else a generic one for its material. */
function guessProfile(list: NamedProfile[], tray: Tray): string {
  const byId = tray.info_idx ? list.find((f) => f.filament_id === tray.info_idx) : undefined;
  const kind = (tray.type || "PLA").toUpperCase();
  const sameKind = list.filter((f) => (f.filament_type ?? "").toUpperCase() === kind);
  return (
    byId?.name ??
    sameKind.find((f) => f.name.startsWith(`Generic ${kind}`))?.name ??
    sameKind.find((f) => f.name.startsWith("Bambu"))?.name ??
    sameKind[0]?.name ??
    list.find((f) => f.name.startsWith("Generic PLA"))?.name ??
    list[0]?.name ??
    ""
  );
}

/**
 * The tray chosen for one filament, as a button that opens a list of the
 * loaded trays, each shown with its spool colour.
 */
export function TrayPicker({
  title,
  trays,
  value,
  onChange,
  none,
  extra = [],
  onManage,
  spools,
}: {
  title: string;
  trays: Tray[];
  /** Spoolman spools by tray id. */
  spools?: Record<string, SpoolmanSpool> | null;
  value: number;
  onChange: (id: number) => void;
  /** Label for "no tray" (e.g. "Choose later"); omit to not offer it. */
  none?: string;
  /** More choices that aren't loaded trays, e.g. the external spool. */
  extra?: { id: number; label: string }[];
  /** "Check the trays" from inside the list. */
  onManage?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const chosen = trays.find((t) => t.id === value) ?? null;
  const chosenExtra = extra.find((x) => x.id === value) ?? null;
  const pick = (id: number) => {
    onChange(id);
    setOpen(false);
  };
  return (
    <>
      <button type="button" className="map-target tray-button" onClick={() => setOpen(true)} aria-label={tr("{0}: {1}. Change", [title, chosen ? trayLabel(chosen) : chosenExtra?.label ?? none ?? tr("none")])}>
        <div>
          <div style={{ fontWeight: 620 }}>{chosen ? chosen.name ?? chosen.type : chosenExtra?.label ?? none ?? tr("Choose")}</div>
          <div className="faint">{chosen ? trayLabel(chosen) : tr("no tray")}</div>
        </div>
        <Spool color={chosen?.color ?? ""} empty={!chosen} size={36} />
      </button>
      {open && (
        <Sheet title={title} onClose={() => setOpen(false)}>
          <ul className="tray-list" role="listbox" aria-label={title}>
            {trays.map((t) => (
              <li key={t.id}>
                <button type="button" className="tray-row tray-option" role="option" aria-selected={t.id === value} onClick={() => pick(t.id)}>
                  <Spool color={t.color} size={40} label={`${t.type}, ${t.color}`} />
                  <div className="grow" style={{ minWidth: 0, textAlign: "left" }}>
                    <div style={{ fontWeight: 620 }}>{t.name ?? t.type}</div>
                    <div className="faint">
                      {trayLabel(t)}
                      {t.remain != null && t.remain >= 0 && !spools?.[t.id] ? tr(", {0}% left", [t.remain]) : ""}
                    </div>
                    {spools?.[t.id] && <div className="faint small">{spoolLine(spools[t.id])}</div>}
                  </div>
                  {t.id === value && <span className="tray-check" aria-hidden="true">✓</span>}
                </button>
              </li>
            ))}
            {extra.map((x) => (
              <li key={x.id}>
                <button type="button" className="tray-row tray-option" role="option" aria-selected={x.id === value} onClick={() => pick(x.id)}>
                  <Spool color="" empty size={40} />
                  <div className="grow" style={{ textAlign: "left", fontWeight: 620 }}>{x.label}</div>
                  {x.id === value && <span className="tray-check" aria-hidden="true">✓</span>}
                </button>
              </li>
            ))}
            {none && (
              <li>
                <button type="button" className="tray-row tray-option" role="option" aria-selected={value < 0} onClick={() => pick(-1)}>
                  <Spool color="" empty size={40} />
                  <div className="grow" style={{ textAlign: "left" }}>{none}</div>
                  {value < 0 && <span className="tray-check" aria-hidden="true">✓</span>}
                </button>
              </li>
            )}
          </ul>
          {onManage && (
            <button
              type="button"
              className="btn block"
              onClick={() => {
                setOpen(false);
                onManage();
              }}
            >
              {tr("Wrong spool? Check the trays")}
            </button>
          )}
        </Sheet>
      )}
    </>
  );
}
