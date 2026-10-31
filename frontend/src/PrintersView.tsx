import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { api } from "./api";
import type { FtpDataMode, JobRef, MachineProfile, Printer, ProfileChoices, QueueEntry, Tray } from "./types";
import { TraysSheet, usePlates } from "./Trays";
import { BOOT_SECS, bootState, CameraView, Notice, Sheet, fmtLeft, Spool, fmtDuration, isBusy, printerState, timeAgo, toast, trayLabel, usePoll, remoteImg } from "./ui";
import { lang, locale, tr } from "./i18n";

const SPEEDS: [number, string][] = [
  [1, tr("Silent")],
  [2, tr("Standard")],
  [3, tr("Sport")],
  [4, tr("Ludicrous")],
];

/** Pause/resume/stop and speed while printing, light and refresh always. */
export function PrintControls({ printer, onDone, showRefresh = true }: { printer: Printer; onDone?: () => void | Promise<void>; showRefresh?: boolean }) {
  const s = printer.status;
  const busy = isBusy(s);
  const [pending, setPending] = useState("");
  async function control(action: string, confirmText?: string) {
    if (confirmText && !window.confirm(confirmText)) return;
    setPending(action);
    try {
      await api.post(`printers/${printer.id}/control`, { action });
      setTimeout(() => void onDone?.(), 800);
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setPending("");
    }
  }
  if (!s.connected) return null;
  return (
    <>
      {s.state === "PAUSE" && (
        <button className="btn small" disabled={!!pending} onClick={() => control("resume")}>
          {tr("Resume")}
        </button>
      )}
      {busy && s.state !== "PAUSE" && (
        <button className="btn small" disabled={!!pending} onClick={() => control("pause")}>
          {tr("Pause")}
        </button>
      )}
      {busy && (
        <button className="btn small danger" disabled={!!pending} onClick={() => control("stop", tr("Stop the print on {0}? This can't be undone.", [printer.name]))}>
          {tr("Stop")}
        </button>
      )}
      <button className="btn small" disabled={!!pending} aria-pressed={!!s.chamber_light} onClick={() => control(s.chamber_light ? "light_off" : "light_on")}>
        {s.chamber_light ? tr("Light off") : tr("Light on")}
      </button>
      {showRefresh && (
        <button className="btn small ghost" disabled={!!pending} onClick={() => control("refresh")} aria-label={tr("Refresh {0}", [printer.name])}>
          {pending === "refresh" ? tr("Refreshing…") : tr("Refresh")}
        </button>
      )}
      {busy && (
        <label className="speed-select">
          <span className="faint small">{tr("Speed")}</span>
          <select
            className="input small"
            value={s.speed_level ?? ""}
            disabled={!!pending}
            onChange={(e) => control(`speed_${e.target.value}`)}
            aria-label={tr("Print speed")}
          >
            {s.speed_level == null && <option value="">–</option>}
            {SPEEDS.map(([level, label]) => (
              <option key={level} value={level}>
                {label}
              </option>
            ))}
          </select>
        </label>
      )}
    </>
  );
}

export function PrintersView({ printers, reload }: { printers: Printer[] | null; reload: () => Promise<void> }) {
  const [queue, setQueue] = useState<QueueEntry[]>([]);
  const loadQueue = async () => {
    try {
      setQueue(await api.get<QueueEntry[]>("queue"));
    } catch {
      /* keep last */
    }
  };
  usePoll(loadQueue, 4000);
  const refresh = async () => {
    await Promise.all([reload(), loadQueue()]);
  };

  return (
    <main className="page">
      <h1 className="page-title">{tr("Printers")}</h1>

      {printers && printers.length === 0 && (
        <div className="empty panel">
          <p>{tr("Add your Bambu Lab printer to send prints to it over your network.")}</p>
          <p className="faint">
            {tr("On the printer, turn on LAN Only mode and Developer mode, then note the IP address, serial number and access code.")}
          </p>
          <a className="btn primary" href="#/settings/printers/new">
            {tr("Add printer")}
          </a>
        </div>
      )}

      {printers?.map((p) => (
        <PrinterCard key={p.id} printer={p} queue={queue.filter((q) => q.group === p.machine_profile)} reload={refresh} />
      ))}
    </main>
  );
}

/** Settings → Printers: add, edit, remove and choose the default printer.
 * `open` is the printer being edited (its id, or "new"), from the address. */
export function PrinterSettings({ printers, reload, open }: { printers: Printer[] | null; reload: () => Promise<void>; open: string | null }) {
  const editing: Printer | "new" | null = open === "new" ? "new" : (printers?.find((p) => p.id === open) ?? null);
  const close = () => {
    window.location.hash = "#/settings/printers";
  };
  async function makeDefault(p: Printer) {
    try {
      await api.post(`printers/${p.id}/default`);
      await reload();
      toast(tr("{0} is the default printer now", [p.name]));
    } catch (e) {
      toast((e as Error).message);
    }
  }
  return (
    <>
      {printers && printers.length === 0 && (
        <div className="panel">
          <p style={{ marginTop: 0 }}>{tr("Add your Bambu Lab printer to send prints to it over your network.")}</p>
          <p className="faint" style={{ marginBottom: 0 }}>
            {tr("On the printer, turn on LAN Only mode and Developer mode, then note the IP address, serial number and access code.")}
          </p>
        </div>
      )}
      {printers && printers.length > 0 && (
        <ul className="notify-list">
          {printers.map((p) => (
            <li key={p.id} className={`notify-item ${p.enabled ? "" : "off"}`}>
              <div className="row" style={{ gap: 8 }}>
                <span className="grow" style={{ minWidth: 0 }}>
                  <b className="ellipsis" style={{ display: "block" }}>
                    <span className={`dot ${p.status.connected ? "ok" : ""}`} style={{ display: "inline-block", verticalAlign: "middle", marginRight: 4 }} aria-hidden />
                    {p.name}
                  </b>
                  <small className="faint ellipsis" style={{ display: "block" }}>
                    {[p.machine_profile.replace(/^Bambu Lab /, ""), p.host, p.is_default ? tr("default") : null, p.enabled ? null : tr("not connected")].filter(Boolean).join(" · ")}
                  </small>
                </span>
                {!p.is_default && (
                  <button type="button" className="btn small ghost" onClick={() => makeDefault(p)}>
                    {tr("Make default")}
                  </button>
                )}
                <a className="btn small" href={`#/settings/printers/${p.id}`}>
                  {tr("Edit")}
                </a>
              </div>
            </li>
          ))}
        </ul>
      )}
      <a className="btn block" href="#/settings/printers/new" style={{ marginTop: 12 }}>
        {tr("Add printer")}
      </a>
      {editing && (
        <PrinterForm
          printer={editing === "new" ? null : editing}
          onClose={close}
          onSaved={async () => {
            close();
            await reload();
          }}
        />
      )}
    </>
  );
}

/** The picture for a job: the photo of the result, else the plate, else the model. */
function jobPicture(j: JobRef, preferPhoto: boolean): string | null {
  if (preferPhoto && j.photo) return `api/jobs/${j.id}/photo.jpg`;
  if (j.thumbnail_plate) return `api/jobs/${j.id}/plates/${j.thumbnail_plate}/thumbnail`;
  return remoteImg(j.cover) ?? null;
}

const OUTCOME_TEXT: Record<string, string> = {
  finished: tr("Printed"),
  failed: tr("Failed"),
  cancelled: tr("Stopped"),
  unknown: tr("Result unknown"),
};

const PROBLEM_TEXT: Record<string, string> = {
  fatal: tr("The printer reports a fatal error."),
  serious: tr("The printer reports a serious error."),
  common: tr("The printer reports an error."),
  info: tr("The printer has a message."),
};

/** "1:05" */
function fmtSecs(secs: number) {
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
}

function temp(now: number | null | undefined, target: number | null | undefined) {
  if (now == null) return "–";
  const t = target != null && target > 0 ? Math.round(target) : null;
  return t != null && Math.abs(now - t) > 2 ? `${Math.round(now)}° → ${t}°` : `${Math.round(now)}°`;
}

function PrinterCard({
  printer,
  queue,
  reload,
}: {
  printer: Printer;
  queue: QueueEntry[];
  reload: () => Promise<void>;
}) {
  const settingsLink = `#/settings/printers/${printer.id}`;
  const s = printer.status;
  const st = printerState(s, printer);
  const busy = isBusy(s);
  const [showTrays, setShowTrays] = useState(false);
  // The camera shows by itself during a print; otherwise on request.
  const [cameraOn, setCameraOn] = useState<boolean | null>(null);
  const showCamera = !!printer.camera && s.connected && (cameraOn ?? busy);
  const menu = useRef<HTMLDetailsElement>(null);
  const job = busy ? printer.current_job : null;
  const plugOff = printer.power?.on === false;
  // Messages hidden on this device; one comes back when it goes away and
  // the printer reports it again.
  const hiddenKey = `hms-hidden:${printer.id}`;
  const [hidden, setHidden] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(hiddenKey) ?? "[]");
    } catch {
      return [];
    }
  });
  const active = (s.problems ?? []).map((p) => p.code);
  useEffect(() => {
    if (!s.connected) return;
    const still = hidden.filter((c) => active.includes(c));
    if (still.length !== hidden.length) store(still);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active.join(","), s.connected]);
  function store(list: string[]) {
    setHidden(list);
    try {
      localStorage.setItem(hiddenKey, JSON.stringify(list));
    } catch {
      /* private mode */
    }
  }
  const hide = (code: string) => store([...hidden, code]);
  const boot = bootState(printer);
  const [switching, setSwitching] = useState(false);
  async function switchPlug(on: boolean) {
    if (menu.current) menu.current.open = false;
    if (!on && !window.confirm(tr("Turn off {0} at the plug?", [printer.name]))) return;
    setSwitching(true);
    try {
      await api.post(`printers/${printer.id}/power`, { on });
      await reload();
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setSwitching(false);
    }
  }
  const last = !busy ? printer.last_job : null;

  // Trays grouped per AMS unit; the external spool last.
  const units = (s.ams_units ?? []).map((u) => ({ unit: u, trays: s.trays.filter((t) => t.ams === u.id) }));
  const loose = s.trays.filter((t) => !(s.ams_units ?? []).some((u) => u.id === t.ams));

  async function action(fn: () => Promise<unknown>) {
    if (menu.current) menu.current.open = false;
    try {
      await fn();
      await reload();
    } catch (e) {
      toast((e as Error).message);
    }
  }

  const spool = (t: Tray) => (
    <button
      key={t.id}
      type="button"
      className={`rail-item ${t.active ? "active" : ""}`}
      title={t.name ?? undefined}
      onClick={() => setShowTrays(true)}
      aria-label={tr("{0}: {1}. Change trays", [trayLabel(t), t.type || tr("empty")])}
    >
      <Spool color={t.color} empty={!t.type} size={42} label={`${trayLabel(t)}: ${t.type || tr("empty")}`} />
      <span className="faint">
        {t.type || tr("Empty")}
        <br />
        {t.id === 254 ? tr("Ext.") : t.ams != null && t.ams >= 128 ? "HT" : `${(t.slot ?? 0) + 1}`}
        {t.remain != null ? ` · ${t.remain}%` : ""}
      </span>
      {t.remain != null && (
        <span className="remain-bar" aria-hidden>
          <span style={{ width: `${Math.max(3, Math.min(100, t.remain))}%` }} />
        </span>
      )}
    </button>
  );

  return (
    <article className={`printer-card tone-${st.tone || "none"}`}>
      <header className="pc-head">
        <div className="grow" style={{ minWidth: 0 }}>
          <h2 className="printer-name ellipsis">{printer.name}</h2>
          <div className="faint small pc-sub">
            {[printer.printer_model.replace(/^Bambu Lab /, ""), s.nozzle_diameter ? tr("{0} mm nozzle", [s.nozzle_diameter]) : null, printer.is_default ? tr("default") : null]
              .filter(Boolean)
              .join(" · ")}
          </div>
        </div>
        <span className={`pill ${st.tone}`}>
          <span className={`dot ${st.tone}`} />
          {st.text}
          {busy && s.percent != null ? ` · ${s.percent}%` : ""}
        </span>
        <details className="menu" ref={menu}>
          <summary className="btn small menu-btn" aria-label={tr("More for {0}", [printer.name])}>
            <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
              <circle cx="5" cy="12" r="2" fill="currentColor" />
              <circle cx="12" cy="12" r="2" fill="currentColor" />
              <circle cx="19" cy="12" r="2" fill="currentColor" />
            </svg>
          </summary>
          <div className="menu-pop" role="menu">
            <a role="menuitem" href={settingsLink}>
              {tr("Printer settings")}
            </a>
            {s.connected && (
              <button type="button" role="menuitem" onClick={() => action(() => api.post(`printers/${printer.id}/control`, { action: "refresh" }))}>
                {tr("Refresh")}
              </button>
            )}
            {printer.power?.on === true && !busy && (
              <button type="button" role="menuitem" onClick={() => switchPlug(false)}>
                {tr("Turn off at the plug")}
              </button>
            )}
          </div>
        </details>
      </header>

      {plugOff && (
        <div className="pc-plug row">
          <span className="grow">{tr("{0} is switched off at the plug.", [printer.name])}</span>
          <button className="btn small primary" disabled={switching} onClick={() => switchPlug(true)}>
            {switching ? tr("Turning on…") : tr("Turn on")}
          </button>
        </div>
      )}
      {boot?.booting && (
        <div className="pc-plug" role="status">
          <div className="row small">
            <span className="grow">{tr("{0} is starting up. Give it about 2 minutes.", [printer.name])}</span>
            <b className="num">{fmtSecs(boot.left)}</b>
          </div>
          <div className="strip-track">
            <div className="strip-fill" style={{ width: `${Math.min(100, ((BOOT_SECS - boot.left) / BOOT_SECS) * 100)}%` }} />
          </div>
        </div>
      )}
      {boot?.late && (
        <Notice tone="err">
          {tr("{0} has had power for {1} but still isn't answering. Check the printer's screen and that it's on the network, or turn the plug off and on again.", [
            printer.name,
            fmtDuration(boot.elapsed),
          ])}{" "}
          {s.last_error ? <span className="small">({s.last_error})</span> : null}
        </Notice>
      )}
      {!plugOff && !boot && !s.connected && s.last_error !== "disabled" && (
        <Notice tone="err">
          {s.last_update ? tr("{0} isn't answering. Last heard {1}.", [printer.name, timeAgo(s.last_update)]) : tr("{0} isn't answering.", [printer.name])}{" "}
          {s.last_error ? <span className="small">({s.last_error})</span> : null}
        </Notice>
      )}
      {s.connected &&
        s.problems
          ?.filter((p) => !hidden.includes(p.code))
          .map((p) => {
            const text = p.texts?.[lang] ?? p.text;
            return (
              <Notice key={p.code} tone={p.severity === "fatal" || p.severity === "serious" ? "err" : undefined}>
                <span className="problem">
                  <span className="grow">
                    {text ? `${text[0].toUpperCase()}${text.slice(1).replace(/\.?$/, ".")}` : PROBLEM_TEXT[p.severity]}{" "}
                    <a href={p.url} target="_blank" rel="noreferrer">
                      {p.code}
                    </a>
                  </span>
                  <button type="button" className="problem-hide" onClick={() => hide(p.code)} aria-label={tr("Hide this message")} title={tr("Hide this message")}>
                    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                      <path d="M3.5 3.5l9 9M12.5 3.5l-9 9" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
                    </svg>
                  </button>
                </span>
              </Notice>
            );
          })}

      {busy && (
        <section className="pc-job">
          <div className="pc-job-head">
            {job && jobPicture(job, false) ? <img src={jobPicture(job, false)!} alt="" /> : <div className="pc-job-ph" aria-hidden />}
            <div style={{ minWidth: 0 }} className="grow">
              {job ? (
                <a className="pc-job-title ellipsis" href={`#/job/${job.id}`}>
                  {job.title}
                </a>
              ) : (
                <div className="pc-job-title ellipsis">{s.task_name || tr("A print from another app")}</div>
              )}
              {job?.plate_label && <div className="faint small ellipsis">{job.plate_label[0].toUpperCase() + job.plate_label.slice(1)}</div>}
            </div>
            <b className="pc-percent num">{s.percent ?? 0}%</b>
          </div>
          <div className="strip-track pc-track">
            <div className="strip-fill" style={{ width: `${Math.min(100, s.percent ?? 0)}%` }} />
          </div>
          <div className="faint small num" style={{ marginTop: 6 }}>
            {fmtLeft(s.remaining_minutes)}
            {s.layer != null && s.total_layers ? tr(", layer {0} of {1}", [s.layer, s.total_layers]) : ""}
          </div>
        </section>
      )}

      {last && s.connected && (
        <a className="pc-last" href={`#/job/${last.id}`}>
          {jobPicture(last, true) ? <img src={jobPicture(last, true)!} alt="" loading="lazy" /> : <div className="pc-job-ph" aria-hidden />}
          <span style={{ minWidth: 0 }} className="grow">
            <span className="faint small">{tr("Last print")}</span>
            <span className="ellipsis pc-last-title">{last.title}</span>
            <span className={`small outcome-${last.outcome ?? "unknown"}`}>
              {OUTCOME_TEXT[last.outcome ?? "unknown"]}
              {last.ended_at ? ` · ${timeAgo(last.ended_at)}` : ""}
            </span>
          </span>
        </a>
      )}

      {showCamera && <CameraView printerId={printer.id} />}
      {busy && s.connected && !printer.camera && (
        <a className="linkish small" style={{ marginTop: 8, display: "inline-block" }} href={settingsLink}>
          {tr("Set up the camera to watch the print here")}
        </a>
      )}

      {s.connected && (
        <div className="controls">
          <PrintControls printer={printer} onDone={reload} showRefresh={false} />
          {printer.camera && (
            <button className="btn small" onClick={() => setCameraOn(!showCamera)} aria-pressed={showCamera}>
              {showCamera ? tr("Hide camera") : tr("Camera")}
            </button>
          )}
          {!busy && (
            <a className="btn small primary" href="#/">
              {tr("Print something")}
            </a>
          )}
        </div>
      )}

      {s.connected && (
        <div className="pc-stats">
          <div>
            <span>{tr("Nozzle")}</span>
            <b className="num">{temp(s.nozzle_temp, s.nozzle_target)}</b>
          </div>
          <div>
            <span>{tr("Bed")}</span>
            <b className="num">{temp(s.bed_temp, s.bed_target)}</b>
          </div>
          {s.fan_part != null && (
            <div title={tr("Part cooling {0} %, auxiliary {1} %, chamber {2} %", [s.fan_part, s.fan_aux ?? "–", s.fan_chamber ?? "–"])}>
              <span>{tr("Fan")}</span>
              <b className="num">{s.fan_part ? `${s.fan_part} %` : tr("Off")}</b>
            </div>
          )}
          <a className="pc-more" href={`#/printers/${printer.id}/sensors`} aria-label={tr("All sensors of {0}", [printer.name])}>
            <span>{tr("Sensors")}</span>
            <b>{tr("More ›")}</b>
          </a>
        </div>
      )}

      {(units.length > 0 || loose.length > 0 || s.external) && (
        <section className="pc-filament" aria-label={tr("Loaded filament")}>
          {units.map(({ unit, trays }, i) => (
            <div key={unit.id} className="pc-unit">
              <div className="pc-unit-head small">
                <b>{unit.name}</b>
                {unit.humidity_pct != null && <span className="faint">{tr("Humidity {0} %", [unit.humidity_pct])}</span>}
                {unit.temp != null && <span className="faint num">{Math.round(unit.temp)} °C</span>}
                {i === 0 && (
                  <button type="button" className="linkish small pc-trays-link" onClick={() => setShowTrays(true)}>
                    {tr("Change trays")}
                  </button>
                )}
              </div>
              <div className="rail">{trays.map(spool)}</div>
            </div>
          ))}
          {units.length > 0 && loose.length === 0 && s.external && !s.external.type ? (
            // With an AMS, an empty external spool holder isn't worth a line.
            null
          ) : (
            (loose.length > 0 || s.external) && (
              <div className="pc-unit">
                {units.length > 0 && (
                  <div className="pc-unit-head small">
                    <b>{tr("External spool")}</b>
                  </div>
                )}
                <div className="rail">
                  {loose.map(spool)}
                  {s.external && spool(s.external)}
                </div>
              </div>
            )
          )}
          {units.length === 0 && (
            <button type="button" className="linkish small" onClick={() => setShowTrays(true)}>
              {tr("Change trays")}
            </button>
          )}
        </section>
      )}
      {showTrays && <TraysSheet printer={printer} onClose={() => setShowTrays(false)} />}

      {queue.length > 0 && <QueuePanel printer={printer} queue={queue} reload={reload} />}

      <p className="faint small" style={{ margin: "14px 0 0" }}>
        {printer.host}
        {s.last_update ? tr(", updated {0}", [timeAgo(s.last_update)]) : ""}
        {printer.power?.on && printer.power.off_in != null
          ? tr(", plug turns off at {0}", [new Date(Date.now() + printer.power.off_in * 1000).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" })])
          : ""}
      </p>
    </article>
  );
}

/** What's next for this printer's type: one line, started from here; the
 * whole queue (order, removing) is on the print page. */
function QueuePanel({ printer, queue, reload }: { printer: Printer; queue: QueueEntry[]; reload: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const free = printer.status.connected && !isBusy(printer.status);
  const next = queue[0];
  const name = next.plate_label ? `${next.title}, ${next.plate_label}` : next.title;

  async function start() {
    if (!window.confirm(tr("Start “{0}” on {1}? Make sure the build plate is clear.", [name, printer.name]))) return;
    setBusy(true);
    try {
      await api.post(`printers/${printer.id}/start-next`);
      toast(tr("Starting the next print"));
      await reload();
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="pc-next" aria-label={tr("Queue")}>
      <a className="pc-next-item" href={`#/job/${next.job_id}`}>
        {next.has_thumbnail && next.plate ? (
          <img className="plate-thumb" src={`api/jobs/${next.job_id}/plates/${next.plate}/thumbnail`} alt="" loading="lazy" />
        ) : next.cover ? (
          <img src={remoteImg(next.cover)} alt="" loading="lazy" />
        ) : (
          <div className="pc-job-ph" aria-hidden />
        )}
        <span style={{ minWidth: 0 }} className="grow">
          <span className="faint small">{tr("Next in the queue")}</span>
          <span className="ellipsis pc-last-title">{name}</span>
          <span className={`small ${next.filament_ok ? "faint" : "queue-warn"}`}>
            {next.filament_ok ? fmtDuration(next.prediction_secs) : tr("Filament not loaded — tap to choose trays")}
          </span>
        </span>
      </a>
      {free && (
        <button className="btn small primary" disabled={busy || !next.filament_ok} onClick={start}>
          {tr("Start")}
        </button>
      )}
      {queue.length > 1 && (
        <a className="pc-next-more small" href="#/">
          {tr("{0} more in the queue ›", [queue.length - 1])}
        </a>
      )}
    </section>
  );
}

type FormState = {
  name: string;
  host: string;
  serial: string;
  access_code: string;
  machine_profile: string;
  process_profile: string;
  filament_profile: string;
  ftp_data_mode: FtpDataMode;
  plates: string[];
  default_plate: string;
  camera: Printer["camera"];
  camera_entity: string;
  power_switch: string;
  power_off_after_min: number;
  enabled: boolean;
};

function PrinterForm({ printer, onClose, onSaved }: { printer: Printer | null; onClose: () => void; onSaved: () => void }) {
  const [f, setF] = useState<FormState>({
    name: printer?.name ?? "",
    host: printer?.host ?? "",
    serial: printer?.serial ?? "",
    access_code: "",
    machine_profile: printer?.machine_profile ?? "",
    process_profile: printer?.process_profile ?? "",
    filament_profile: printer?.filament_profile ?? "",
    ftp_data_mode: printer?.ftp_data_mode ?? "auto",
    plates: printer?.plates ?? [],
    default_plate: printer?.default_plate ?? "",
    camera: printer?.camera ?? "",
    camera_entity: printer?.camera_entity ?? "",
    power_switch: printer?.power_switch ?? "",
    power_off_after_min: printer?.power_off_after_min ?? 0,
    enabled: printer?.enabled ?? true,
  });
  const [usePlug, setUsePlug] = useState(!!printer?.power_switch);
  const [haSwitches, setHaSwitches] = useState<{ entity_id: string; name: string }[] | null>(null);
  const [haSwitchesError, setHaSwitchesError] = useState<string | null>(null);
  useEffect(() => {
    if (!usePlug || haSwitches) return;
    api
      .get<{ entity_id: string; name: string }[]>("ha/switches")
      .then(setHaSwitches)
      .catch((e) => setHaSwitchesError((e as Error).message));
  }, [usePlug]);
  const [haCameras, setHaCameras] = useState<{ entity_id: string; name: string }[] | null>(null);
  const [haCamerasError, setHaCamerasError] = useState<string | null>(null);
  useEffect(() => {
    if (f.camera !== "ha" || haCameras) return;
    api
      .get<{ entity_id: string; name: string }[]>("ha/cameras")
      .then(setHaCameras)
      .catch((e) => setHaCamerasError((e as Error).message));
  }, [f.camera]);
  const allPlates = usePlates();
  const togglePlate = (value: string, on: boolean) =>
    setF((x) => {
      const plates = on ? [...x.plates, value] : x.plates.filter((p) => p !== value);
      // The default must stay one of the plates; a single plate is the default.
      const default_plate = plates.includes(x.default_plate) ? x.default_plate : plates.length === 1 ? plates[0] : "";
      return { ...x, plates, default_plate };
    });
  const [machines, setMachines] = useState<MachineProfile[] | null>(null);
  const [machinesError, setMachinesError] = useState<string | null>(null);
  const [choices, setChoices] = useState<ProfileChoices | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  type Check = { ok: boolean; error: string | null };
  const [test, setTest] = useState<{ mqtt: Check; ftp: Check; commands?: Check } | null>(null);

  useEffect(() => {
    api
      .get<MachineProfile[]>("profiles/machines")
      .then(setMachines)
      .catch((e) => setMachinesError((e as Error).message));
  }, []);

  useEffect(() => {
    if (!f.machine_profile || !machines) return;
    api
      .get<ProfileChoices>(`profiles?machine=${encodeURIComponent(f.machine_profile)}`)
      .then(setChoices)
      .catch(() => setChoices(null));
  }, [f.machine_profile, machines]);

  const models = useMemo(() => {
    const groups = new Map<string, MachineProfile[]>();
    for (const m of machines ?? []) {
      const key = m.printer_model || tr("Other");
      groups.set(key, [...(groups.get(key) ?? []), m]);
    }
    return [...groups.entries()];
  }, [machines]);

  const set = <K extends keyof FormState>(k: K, v: FormState[K]) => setF((x) => ({ ...x, [k]: v }));

  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const body = {
        ...f,
        access_code: f.access_code || undefined,
        default_plate: f.default_plate || undefined,
        camera_entity: f.camera === "ha" ? f.camera_entity.trim() || undefined : undefined,
        power_switch: usePlug ? f.power_switch.trim() || undefined : undefined,
        power_off_after_min: usePlug ? f.power_off_after_min : 0,
      };
      const saved = printer ? await api.put<Printer>(`printers/${printer.id}`, body) : await api.post<Printer>("printers", body);
      toast(printer ? tr("Printer saved") : tr("Printer added"));
      if (!printer) {
        // Run a connection test straight away so problems show up now.
        try {
          await new Promise((r) => setTimeout(r, 1500));
          const t = await api.post<typeof test>(`printers/${saved.id}/test`);
          if (t && (!t.mqtt.ok || !t.ftp.ok || t.commands?.ok === false)) {
            toast(tr("Added, but the connection test failed. Open Edit to see why."));
          }
        } catch {
          /* ignore */
        }
      }
      onSaved();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function runTest() {
    if (!printer) return;
    setTest(null);
    setBusy(true);
    try {
      setTest(await api.post(`printers/${printer.id}/test`));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!printer || !window.confirm(tr("Remove {0}?", [printer.name]))) return;
    await api.del(`printers/${printer.id}`);
    toast(tr("Printer removed"));
    onSaved();
  }

  return (
    <Sheet title={printer ? tr("Edit {0}", [printer.name]) : tr("Add printer")} onClose={onClose}>
      <form onSubmit={save}>
        {!printer && (
          <Notice>
            {tr("On the printer: Settings, then turn on")} <b>{tr("LAN Only Mode")}</b> {tr("and")} <b>{tr("Developer Mode")}</b>{tr(". The access code is shown on the same screen. The serial number is under Settings, Device.")}
          </Notice>
        )}
        <label className="field">
          <span>{tr("Name")}</span>
          <input className="input" value={f.name} onChange={(e) => set("name", e.target.value)} placeholder={tr("Workshop P1S")} required />
        </label>
        <label className="field">
          <span>{tr("IP address")}</span>
          <input className="input" value={f.host} onChange={(e) => set("host", e.target.value)} placeholder="192.168.1.50" inputMode="decimal" autoCapitalize="off" required />
          <small>{tr("Give the printer a fixed IP in your router so this stays valid.")}</small>
        </label>
        <label className="field">
          <span>{tr("Serial number")}</span>
          <input className="input" value={f.serial} onChange={(e) => set("serial", e.target.value.toUpperCase())} placeholder="01P00A000000000" autoCapitalize="characters" required />
        </label>
        <label className="field">
          <span>{tr("Access code")}</span>
          <input
            className="input"
            value={f.access_code}
            onChange={(e) => set("access_code", e.target.value.trim())}
            placeholder={printer?.has_access_code ? tr("Saved — leave empty to keep") : "8 characters"}
            autoCapitalize="off"
            autoComplete="off"
            required={!printer}
          />
        </label>

        <label className="field">
          <span>{tr("Printer model and nozzle")}</span>
          {machines ? (
            <select className="input" value={f.machine_profile} onChange={(e) => set("machine_profile", e.target.value)} required>
              <option value="" disabled>
                {tr("Choose…")}
              </option>
              {models.map(([model, list]) => (
                <optgroup key={model} label={model}>
                  {list.map((m) => (
                    <option key={m.name} value={m.name}>
                      {m.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          ) : (
            <input
              className="input"
              value={f.machine_profile}
              onChange={(e) => set("machine_profile", e.target.value)}
              placeholder={tr("Bambu Lab P1S 0.4 nozzle")}
              required
            />
          )}
          {machinesError && <small>{tr("Slicer profiles aren't available ({0}). Type the profile name exactly.", [machinesError])}</small>}
          {f.machine_profile && !/\bP1[SP]\b/i.test(f.machine_profile) && (
            <small>
              {/\bX1/i.test(f.machine_profile)
                ? tr("Not tried on this model yet, and its camera (RTSP) isn't supported. If something goes wrong, send Settings → About → Copy diagnostics.")
                : tr("Not tried on this model yet. It should work; if something goes wrong, send Settings → About → Copy diagnostics.")}
            </small>
          )}
        </label>

        {choices && (
          <>
            <label className="field">
              <span>{tr("Default quality")}</span>
              <select className="input" value={f.process_profile} onChange={(e) => set("process_profile", e.target.value)}>
                <option value="">{choices.default_process ? tr("Default ({0})", [choices.default_process]) : tr("Default")}</option>
                {choices.processes.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.name}
                  </option>
                ))}
              </select>
              <small>{tr("Used when a file isn't set up for this printer, and for STL files.")}</small>
            </label>
            <label className="field">
              <span>{tr("Default filament for STL files")}</span>
              <select className="input" value={f.filament_profile} onChange={(e) => set("filament_profile", e.target.value)}>
                <option value="">{tr("Bambu PLA Basic")}</option>
                {choices.filaments.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          </>
        )}

        {allPlates.length > 0 && (
          <fieldset className="field plates-field">
            <span>{tr("Build plates")}</span>
            <div className="check-list">
              {allPlates.map((p) => (
                <label key={p.value} className="check">
                  <input type="checkbox" checked={f.plates.includes(p.value)} onChange={(e) => togglePlate(p.value, e.target.checked)} />
                  <span>{p.label}</span>
                </label>
              ))}
            </div>
            <small>{tr("The plates you have for this printer. Slicing offers these, and picks among them when a filament can't go on the plate.")}</small>
          </fieldset>
        )}
        {f.plates.length > 0 && (
          <label className="field">
            <span>{tr("Usually on the printer")}</span>
            <select className="input" value={f.default_plate} onChange={(e) => set("default_plate", e.target.value)}>
              <option value="">{tr("No default (the file's or profile's plate)")}</option>
              {allPlates
                .filter((p) => f.plates.includes(p.value))
                .map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
            </select>
            <small>{tr("Slices are made for this plate unless you choose another one.")}</small>
          </label>
        )}

        <label className="field">
          <span>{tr("Camera")}</span>
          <select className="input" value={f.camera} onChange={(e) => set("camera", e.target.value as FormState["camera"])}>
            <option value="">{tr("None")}</option>
            <option value="printer">{tr("The printer's camera")}</option>
            <option value="ha">{tr("A Home Assistant camera")}</option>
          </select>
          <small>
            {f.camera === "printer"
              ? tr("AirSlice reads the camera itself (P1 and A1 series). X1 cameras aren't supported this way. If Home Assistant's Bambu Lab integration shows the camera too, use that camera instead: the printer allows only one viewer.")
              : f.camera === "ha"
                ? tr("Pictures come from a camera in Home Assistant, such as the Bambu Lab integration's. Outside the add-on, set the Home Assistant address and token under Settings → Notifications.")
                : tr("Shows the print while it runs and keeps a picture of the result.")}
          </small>
        </label>
        {f.camera === "ha" && (
          <label className="field">
            <span>{tr("Home Assistant camera")}</span>
            {haCameras && haCameras.length > 0 ? (
              <select className="input" value={f.camera_entity} onChange={(e) => set("camera_entity", e.target.value)}>
                <option value="">{tr("Choose…")}</option>
                {haCameras.map((c) => (
                  <option key={c.entity_id} value={c.entity_id}>
                    {c.name} ({c.entity_id})
                  </option>
                ))}
              </select>
            ) : (
              <input className="input" value={f.camera_entity} onChange={(e) => set("camera_entity", e.target.value)} placeholder={tr("camera.…")} autoCapitalize="off" />
            )}
            {haCamerasError && <small style={{ color: "var(--err)" }}>{tr("Couldn't list Home Assistant's cameras: {0}", [haCamerasError])}</small>}
          </label>
        )}

        <label className="check" style={{ marginBottom: 10 }}>
          <input type="checkbox" checked={usePlug} onChange={(e) => setUsePlug(e.target.checked)} />
          <span>{tr("Powered through a Home Assistant plug")}</span>
        </label>
        {usePlug && (
          <>
            <label className="field">
              <span>{tr("Plug")}</span>
              {haSwitches && haSwitches.length > 0 ? (
                <select className="input" value={f.power_switch} onChange={(e) => set("power_switch", e.target.value)}>
                  <option value="">{tr("Choose…")}</option>
                  {haSwitches.map((c) => (
                    <option key={c.entity_id} value={c.entity_id}>
                      {c.name} ({c.entity_id})
                    </option>
                  ))}
                </select>
              ) : (
                <input className="input" value={f.power_switch} onChange={(e) => set("power_switch", e.target.value)} placeholder="switch.…" autoCapitalize="off" />
              )}
              <small>
                {haSwitchesError
                  ? tr("Couldn't list Home Assistant's switches: {0}", [haSwitchesError])
                  : tr("AirSlice turns it on when you send a print to the printer while it's off, and shows the power it draws. Its power and energy sensors are found by themselves.")}
              </small>
            </label>
            <label className="field">
              <span>{tr("Turn off after a print")}</span>
              <select className="input" value={f.power_off_after_min} onChange={(e) => set("power_off_after_min", Number(e.target.value))}>
                <option value={0}>{tr("Never")}</option>
                {[...new Set([10, 30, 60, 120, f.power_off_after_min].filter((m) => m > 0))].sort((a, b) => a - b).map((m) => (
                  <option key={m} value={m}>
                    {m < 60 || m % 60 ? tr("{0} min after it's done and cool", [m]) : tr("{0} h after it's done and cool", [m / 60])}
                  </option>
                ))}
              </select>
              <small>{tr("Only when nothing is queued or waiting, and once the nozzle is below 50 °C.")}</small>
            </label>
          </>
        )}

        <details style={{ marginBottom: 14 }}>
          <summary className="small muted">{tr("Advanced")}</summary>
          <label className="field" style={{ marginTop: 10 }}>
            <span>{tr("Upload mode")}</span>
            <select className="input" value={f.ftp_data_mode} onChange={(e) => set("ftp_data_mode", e.target.value as FtpDataMode)}>
              <option value="auto">{tr("Automatic")}</option>
              <option value="private">{tr("Encrypted (X1, P1, P2, H2)")}</option>
              <option value="clear">{tr("Unencrypted data (A1, A1 mini)")}</option>
            </select>
            <small>{tr("Change only if uploads fail.")}</small>
          </label>
          <label className="check">
            <input type="checkbox" checked={f.enabled} onChange={(e) => set("enabled", e.target.checked)} />
            <span>{tr("Connected (turn off to stop talking to this printer)")}</span>
          </label>
        </details>

        {test && (
          <>
            <Notice tone={test.mqtt.ok ? "ok" : "err"}>{tr("Status and commands: {0}", [test.mqtt.ok ? tr("working") : test.mqtt.error ?? tr("not connected")])}</Notice>
            <Notice tone={test.ftp.ok ? "ok" : "err"}>{tr("File upload: {0}", [test.ftp.ok ? tr("working") : test.ftp.error])}</Notice>
            {test.commands?.ok === false && <Notice tone="err">{tr("Starting prints: {0}", [test.commands.error ?? ""])}</Notice>}
          </>
        )}
        {error && <Notice tone="err">{error}</Notice>}

        <div className="row wrap">
          <button className="btn primary grow" disabled={busy}>
            {printer ? tr("Save") : tr("Add printer")}
          </button>
          {printer && (
            <button type="button" className="btn" onClick={runTest} disabled={busy}>
              {tr("Test connection")}
            </button>
          )}
        </div>
        {printer && (
          <button type="button" className="btn danger block" style={{ marginTop: 18 }} onClick={remove}>
            {tr("Remove printer")}
          </button>
        )}
      </form>
    </Sheet>
  );
}
