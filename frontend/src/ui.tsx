import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Printer, Status, Tray } from "./types";
import { locale, tr } from "./i18n";

export function fmtDuration(secs: number | null | undefined): string {
  if (secs == null) return "–";
  const h = Math.floor(secs / 3600);
  const m = Math.max(1, Math.round((secs % 3600) / 60));
  return h > 0 ? tr("{0} h {1} min", [h, m]) : `${m} min`;
}

export function fmtMinutes(min: number | null | undefined): string {
  return min == null ? "–" : fmtDuration(min * 60);
}

/** "12 min left, ready about 15:42" (weekday when it's not today). */
export function fmtLeft(min: number | null | undefined): string {
  if (min == null) return "–";
  const at = new Date(Date.now() + min * 60_000);
  const time = at.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
  const day = at.toDateString() === new Date().toDateString() ? "" : `${at.toLocaleDateString(locale, { weekday: "short" })} `;
  return tr("{0} left, ready about {1}{2}", [fmtMinutes(min), day, time]);
}

export function timeAgo(ts: number | null | undefined): string {
  if (!ts) return "never";
  const s = Math.max(0, Date.now() / 1000 - ts);
  if (s < 60) return tr("just now");
  if (s < 3600) return tr("{0} min ago", [Math.floor(s / 60)]);
  if (s < 86400) return tr("{0} h ago", [Math.floor(s / 3600)]);
  return new Date(ts * 1000).toLocaleDateString(locale);
}

export function fmtSize(bytes: number | null | undefined): string {
  if (!bytes) return "";
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

const STATE_TEXT: Record<string, string> = {
  IDLE: tr("Idle"),
  PREPARE: tr("Preparing"),
  RUNNING: tr("Printing"),
  PAUSE: tr("Paused"),
  FINISH: tr("Finished"),
  FAILED: tr("Failed"),
  SLICING: tr("Slicing"),
};

/** A model picture to show: MakerWorld's go through the server, whose CDN
 * hides pictures from some browsers. */
export function remoteImg(url: string): string;
export function remoteImg(url: string | null | undefined): string | undefined;
export function remoteImg(url: string | null | undefined): string | undefined {
  if (!url) return undefined;
  try {
    // The same rule as the server's (api::jobs::is_makerworld_picture).
    const u = new URL(url);
    if (u.protocol === "https:" && u.hostname.endsWith(".bblmw.com")) return `api/makerworld/image?u=${encodeURIComponent(url)}`;
  } catch {
    /* relative, ours */
  }
  return url;
}

/** How long a printer gets to start after its plug goes on. */
export const BOOT_SECS = 120;

/** After the plug went on: still starting (`left` seconds to go), or late
 * (`elapsed` seconds with power). Disabled printers never connect, so none. */
export function bootState(p: Pick<Printer, "power" | "status">): { booting: boolean; late: boolean; left: number; elapsed: number } | null {
  const elapsed = p.power?.on ? p.power.on_secs : null;
  if (elapsed == null || p.status.connected || p.status.last_error === "disabled") return null;
  return { booting: elapsed < BOOT_SECS, late: elapsed >= BOOT_SECS, left: Math.max(0, Math.round(BOOT_SECS - elapsed)), elapsed };
}

export function printerState(s: Status, p?: Pick<Printer, "power" | "status">): { text: string; tone: "ok" | "busy" | "err" | "" } {
  if (p?.power?.on === false && !s.connected && s.last_error !== "disabled") return { text: tr("Switched off"), tone: "" };
  if (p && bootState(p)?.booting) return { text: tr("Starting up"), tone: "busy" };
  if (!s.connected) return { text: s.last_error === "disabled" ? tr("Disabled") : tr("Offline"), tone: "err" };
  const state = s.state ?? "";
  if (s.print_error && state === "FAILED") return { text: tr("Error"), tone: "err" };
  const text = STATE_TEXT[state] ?? (state ? state.toLowerCase() : tr("Connected"));
  const tone = ["RUNNING", "PREPARE", "PAUSE"].includes(state) ? "busy" : "ok";
  return { text, tone };
}

/** Printing right now. An offline printer's last known state doesn't count. */
export function isBusy(s: Status): boolean {
  return s.connected && ["RUNNING", "PREPARE", "PAUSE", "SLICING"].includes(s.state ?? "");
}

/** Run `fn` now and every `ms` while `active`; returns a manual refresh. */
export function usePoll(fn: () => Promise<void> | void, ms: number, active = true) {
  const saved = useRef(fn);
  saved.current = fn;
  useEffect(() => {
    if (!active) return;
    let stopped = false;
    let timer: number | undefined;
    const tick = async () => {
      try {
        await saved.current();
      } finally {
        if (!stopped) timer = window.setTimeout(tick, document.hidden ? ms * 4 : ms);
      }
    };
    tick();
    const wake = () => {
      if (!document.hidden) {
        window.clearTimeout(timer);
        tick();
      }
    };
    document.addEventListener("visibilitychange", wake);
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
    };
  }, [ms, active]);
}

/** A filament spool seen from the side: the colour ring is the filament. */
export function Spool({ color, size = 40, empty = false, label }: { color: string; size?: number; empty?: boolean; label?: string }) {
  const c = empty || !color ? "transparent" : color;
  return (
    <svg className="spool" width={size} height={size} viewBox="0 0 40 40" role="img" aria-label={label ?? (empty ? tr("Empty slot") : tr("Filament {0}", [color]))}>
      <circle cx="20" cy="20" r="18.5" fill="none" stroke="var(--line)" strokeWidth="1.5" />
      <circle cx="20" cy="20" r="14" fill="none" stroke={c} strokeWidth="9" />
      {empty && <circle cx="20" cy="20" r="14" fill="none" stroke="var(--line)" strokeWidth="1.5" strokeDasharray="3 3" />}
      <circle cx="20" cy="20" r="7.5" fill="var(--panel-2)" stroke="var(--line)" strokeWidth="1" />
      <circle cx="20" cy="20" r="2.5" fill="var(--ink-3)" />
    </svg>
  );
}

export function trayLabel(t: Tray): string {
  if (t.id === 254) return tr("External spool");
  if (t.ams != null && t.ams >= 128) return tr("AMS HT {0}", [t.ams - 127]);
  const unit = t.ams != null ? String.fromCharCode(65 + t.ams) : "";
  return tr("AMS {0}{1}", [unit, (t.slot ?? 0) + 1]);
}

export function Notice({ tone = "", children }: { tone?: "" | "err" | "warn" | "ok"; children: ReactNode }) {
  return <div className={`notice ${tone}`} role={tone === "err" ? "alert" : undefined}>{children}</div>;
}

export function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = "";
    };
  }, [onClose]);
  return (
    <div className="sheet-backdrop" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet" role="dialog" aria-modal="true" aria-label={title}>
        <div className="row" style={{ marginBottom: 12 }}>
          <h2 className="grow" style={{ margin: 0 }}>{title}</h2>
          <button className="btn ghost small" onClick={onClose}>{tr("Close")}</button>
        </div>
        {children}
      </div>
    </div>
  );
}

let toastSetter: ((t: string | null) => void) | null = null;
export function toast(message: string) {
  toastSetter?.(message);
}
export function ToastHost() {
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    toastSetter = setMsg;
    return () => {
      toastSetter = null;
    };
  }, []);
  useEffect(() => {
    if (!msg) return;
    const t = window.setTimeout(() => setMsg(null), 3200);
    return () => window.clearTimeout(t);
  }, [msg]);
  return msg ? <div className="toast" role="status">{msg}</div> : null;
}

export const Icon = {
  print: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M9 3h6v3l-2 3h-2L9 6z" fill="currentColor" stroke="none" />
      <path d="M8 15h8M7 18h10" />
      <path d="M3 21h18" strokeWidth="2.4" />
    </svg>
  ),
  explore: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M15.5 8.5l-2 5-5 2 2-5z" />
    </svg>
  ),
  printers: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="3.5" y="3.5" width="17" height="17" rx="2.5" />
      <path d="M3.5 8h17M9 8v3l3 2 3-2V8" />
      <path d="M7 17h10" />
    </svg>
  ),
  history: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 6h16M4 12h16M4 18h10" />
    </svg>
  ),
  settings: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
      <circle cx="16" cy="7" r="2.2" />
      <circle cx="10" cy="17" r="2.2" />
    </svg>
  ),
};

/** The printer's camera, refreshed every few seconds while on screen. The
 *  next picture loads in the background so the image doesn't flicker. */
export function CameraView({ printerId, every = 3000 }: { printerId: string; every?: number }) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let stop = false;
    let timer: number | undefined;
    async function next() {
      let failed = false;
      try {
        const res = await fetch(`api/printers/${printerId}/camera.jpg?t=${Date.now()}`, { credentials: "same-origin" });
        if (!res.ok) {
          const msg = ((await res.json().catch(() => null)) as { error?: string } | null)?.error;
          throw new Error(msg ?? tr("The camera didn't answer ({0})", [res.status]));
        }
        const url = URL.createObjectURL(await res.blob());
        if (stop) return URL.revokeObjectURL(url);
        setSrc((old) => {
          if (old) URL.revokeObjectURL(old);
          return url;
        });
        setError(null);
      } catch (e) {
        failed = true;
        if (!stop) setError((e as Error).message);
      }
      // Slow down while the camera fails, and pause in a hidden tab.
      if (!stop) timer = window.setTimeout(next, document.hidden ? every * 5 : failed ? every * 3 : every);
    }
    void next();
    return () => {
      stop = true;
      window.clearTimeout(timer);
      setSrc((old) => {
        if (old) URL.revokeObjectURL(old);
        return null;
      });
    };
  }, [printerId, every]);
  return (
    <div className="camera">
      {src ? <img src={src} alt={tr("Printer camera")} /> : !error && <div className="camera-ph faint small">{tr("Connecting to the camera…")}</div>}
      {error && <div className="camera-err small">{error}</div>}
    </div>
  );
}
