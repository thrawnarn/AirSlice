import { useEffect, useState } from "react";
import { api } from "./api";
import { tr } from "./i18n";
import type { Tray } from "./types";
import { Notice, trayLabel } from "./ui";

interface Short {
  tray: number;
  left_g: number;
  need_g: number;
  backups: number[];
  covered: boolean;
  backup_off: boolean;
}

export interface FilamentCheckResult {
  short: Short[];
  unknown: number[];
}

/** Whether the spools last for this job. `mapping` = the trays on screen;
 * without it the server uses the queued trays or its own match. */
export function useFilamentCheck(jobId: string, mapping?: number[] | null): FilamentCheckResult | null {
  const [check, setCheck] = useState<FilamentCheckResult | null>(null);
  const key = mapping?.join() ?? "";
  useEffect(() => {
    let current = true;
    api
      .post<FilamentCheckResult>(`jobs/${jobId}/filament-check`, { ams_mapping: mapping ?? null })
      .then((c) => current && setCheck(c))
      .catch(() => current && setCheck(null));
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId, key]);
  return check;
}

function names(ids: number[], trays: Tray[]) {
  return ids.map((id) => {
    const t = trays.find((x) => x.id === id);
    return t ? trayLabel(t) : `#${id + 1}`;
  });
}

function shortText(s: Short, trays: Tray[]) {
  const [tray] = names([s.tray], trays);
  const args = [tray, Math.round(s.left_g), Math.ceil(s.need_g)];
  const backups = names(s.backups, trays).join(", ");
  if (s.covered) return tr("{0} has about {1} g left and this print needs about {2} g. The printer carries on from {3}.", [...args, backups]);
  if (s.backup_off) return tr("{0} has about {1} g left and this print needs about {2} g. {3} has the same filament, but filament backup is off on the printer.", [...args, backups]);
  if (s.backups.length) return tr("{0} has about {1} g left and this print needs about {2} g. {3} has the same filament, but not enough to finish.", [...args, backups]);
  return tr("{0} has about {1} g left and this print needs about {2} g. No other tray has the same filament.", args);
}

/** The trays that may really run out, as sentences. */
export function filamentWarnings(check: FilamentCheckResult | null, trays: Tray[]): string[] {
  return (check?.short ?? []).filter((s) => !s.covered).map((s) => shortText(s, trays));
}

export function FilamentNotices({ check, trays }: { check: FilamentCheckResult | null; trays: Tray[] }) {
  if (!check) return null;
  return (
    <>
      {check.short.map((s) => (
        <Notice key={s.tray} tone={s.covered ? "" : "warn"}>
          {shortText(s, trays)}
        </Notice>
      ))}
      {check.unknown.length > 0 && (
        <p className="small faint">{tr("AirSlice can't tell how much filament is left in {0}, so it can't warn you if it runs short.", [names(check.unknown, trays).join(", ")])}</p>
      )}
    </>
  );
}
