import { useState } from "react";
import { api } from "./api";
import type { Printer, QueueEntry } from "./types";
import { fmtDuration, isBusy, remoteImg, toast, usePoll } from "./ui";
import { tr } from "./i18n";

/** "Bambu Lab P1S 0.4 nozzle" → "P1S 0.4 nozzle" */
function groupName(profile: string) {
  return profile.replace(/^Bambu Lab /i, "");
}

/** Free to take the next print: online, idle, not switched off. */
function isFree(p: Printer) {
  return p.status.connected && !isBusy(p.status) && p.power?.on !== false;
}

/** The queue on the print page, one group per printer type (what it was
 * sliced for), so a print can go to whichever printer of that type is free. */
export function QueueBoard({ printers }: { printers: Printer[] | null }) {
  const [queue, setQueue] = useState<QueueEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const load = async () => {
    try {
      setQueue(await api.get<QueueEntry[]>("queue"));
    } catch {
      /* keep the last */
    }
  };
  usePoll(load, 5000);
  if (!queue || queue.length === 0) return null;

  async function act(fn: () => Promise<unknown>, done?: string) {
    setBusy(true);
    try {
      await fn();
      if (done) toast(done);
      await load();
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const groups = [...new Set(queue.map((q) => q.group))];
  return (
    <section className="queue-board">
      <h2>{tr("Queue")}</h2>
      {groups.map((g) => {
        const items = queue.filter((q) => q.group === g);
        const total = items.reduce((s, q) => s + (q.prediction_secs ?? 0), 0);
        const ofType = (printers ?? []).filter((p) => p.machine_profile === g);
        const free = ofType.filter(isFree);
        // Free printers of other types: a print can be sliced again for them.
        const others = (printers ?? []).filter((p) => p.machine_profile !== g && isFree(p));
        return (
          <div key={g} className="panel queue-group">
            <div className="row">
              <b className="grow">{groupName(g)}</b>
              <span className="faint small num">
                {items.length === 1 ? tr("1 print") : tr("{0} prints", [items.length])}
                {total ? `, ${fmtDuration(total)}` : ""}
              </span>
            </div>
            <p className="faint small" style={{ margin: "2px 0 8px" }}>
              {free.length
                ? tr("Free now: {0}. Clear the plate before you start.", [free.map((p) => p.name).join(", ")])
                : ofType.length
                  ? tr("No {0} is free right now.", [groupName(g)])
                  : tr("No printer of this type is set up.")}
            </p>
            <ol className="queue">
              {items.map((q, i) => (
                <li key={q.id}>
                  {q.has_thumbnail && q.plate ? (
                    <img className="plate-thumb" src={`api/jobs/${q.job_id}/plates/${q.plate}/thumbnail`} alt="" loading="lazy" />
                  ) : q.cover ? (
                    <img src={remoteImg(q.cover)} alt="" loading="lazy" />
                  ) : (
                    <div className="thumb-ph">{i + 1}</div>
                  )}
                  <a href={`#/job/${q.job_id}`}>
                    <div className="file-name">
                      {i + 1}. {q.title}
                    </div>
                    {q.plate_label && <div className="file-meta ellipsis">{q.plate_label[0].toUpperCase() + q.plate_label.slice(1)}</div>}
                    <div className="file-meta ellipsis">
                      {fmtDuration(q.prediction_secs)}
                      {q.weight_g ? `, ${Math.round(q.weight_g)} g` : ""}
                    </div>
                    {!q.filament_ok && <div className="queue-warn">{tr("Filament not loaded — tap to choose trays")}</div>}
                  </a>
                  <div className="queue-actions">
                    <button className="btn ghost small" aria-label={tr("Move {0} up", [q.title])} disabled={busy || i === 0} onClick={() => act(() => api.post(`queue/${q.id}/move`, { direction: "up" }))}>
                      ↑
                    </button>
                    <button
                      className="btn ghost small"
                      aria-label={tr("Move {0} down", [q.title])}
                      disabled={busy || i === items.length - 1}
                      onClick={() => act(() => api.post(`queue/${q.id}/move`, { direction: "down" }))}
                    >
                      ↓
                    </button>
                    <button className="btn ghost small" aria-label={tr("Remove {0} from the queue", [q.title])} disabled={busy} onClick={() => window.confirm(tr("Remove “{0}” from the queue?", [q.title])) && act(() => api.del(`queue/${q.id}`), tr("Removed from the queue"))}>
                      ✕
                    </button>
                  </div>
                  {(free.length > 0 || (others.length > 0 && !q.presliced)) && (
                    <div className="queue-start">
                      {free.map((p) => (
                        <button
                          key={p.id}
                          className="btn small primary"
                          disabled={busy}
                          onClick={() => {
                            if (!window.confirm(tr("Start “{0}” on {1}? Make sure the build plate is clear.", [q.title, p.name]))) return;
                            void act(() => api.post(`queue/${q.id}/start`, { printer_id: p.id }), tr("Starting on {0}", [p.name]));
                          }}
                        >
                          {tr("Start on {0}", [p.name])}
                        </button>
                      ))}
                      {!q.presliced &&
                        others.map((p) => (
                          <button
                            key={p.id}
                            className="btn small"
                            disabled={busy}
                            onClick={() => {
                              if (!window.confirm(tr("Slice “{0}” again for {1} ({2})? It leaves this queue and joins the {2} queue once it's sliced.", [q.title, p.name, groupName(p.machine_profile)]))) return;
                              void act(() => api.post(`queue/${q.id}/reslice`, { printer_id: p.id }), tr("Slicing for {0}", [p.name]));
                            }}
                          >
                            {tr("Slice for {0}", [p.name])}
                          </button>
                        ))}
                    </div>
                  )}
                </li>
              ))}
            </ol>
          </div>
        );
      })}
    </section>
  );
}
