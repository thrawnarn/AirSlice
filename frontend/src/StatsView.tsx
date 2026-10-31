import { useMemo, useState } from "react";
import { api } from "./api";
import { money } from "./Trays";
import type { Job, Printer } from "./types";
import { fmtDuration, usePoll } from "./ui";
import { locale, tr } from "./i18n";

type Span = "12m" | "year" | "all";
type Metric = "prints" | "hours" | "grams" | "cost";

interface Month {
  key: string; // 2026-10
  label: string;
  long: string;
  prints: number;
  failed: number;
  secs: number;
  grams: number;
  cost: number;
}

/** What a print used: all of it when finished, its share when it stopped. */
function share(j: Job): number {
  const p = j.print;
  if (!p) return 0;
  if (p.outcome === "finished") return 1;
  if (p.outcome === "failed" || p.outcome === "cancelled") return Math.min(100, Math.max(0, p.progress ?? 0)) / 100;
  return 0;
}

const monthKey = (ts: number) => {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

const fmtGrams = (g: number) => (g >= 1000 ? `${(g / 1000).toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })} kg` : `${Math.round(g)} g`);

export function StatsView({ printers }: { printers: Printer[] | null }) {
  const [jobs, setJobs] = useState<Job[] | null>(null);
  const [span, setSpan] = useState<Span>("12m");
  const [metric, setMetric] = useState<Metric>("prints");
  const [printerId, setPrinterId] = useState("");
  const [hover, setHover] = useState<string | null>(null);
  usePoll(async () => {
    try {
      setJobs(await api.get<Job[]>("jobs"));
    } catch {
      /* keep the last list */
    }
  }, 30000);

  const data = useMemo(() => {
    const now = new Date();
    const from =
      span === "12m"
        ? new Date(now.getFullYear(), now.getMonth() - 11, 1).getTime() / 1000
        : span === "year"
          ? new Date(now.getFullYear(), 0, 1).getTime() / 1000
          : 0;
    const ended = (jobs ?? []).filter(
      (j) => j.print?.ended_at && j.print.started_at >= from && (!printerId || j.print.printer_id === printerId) && j.print.outcome && j.print.outcome !== "unknown",
    );
    // Months in order, with empty ones, so gaps show.
    const months = new Map<string, Month>();
    const first = ended.reduce((m, j) => Math.min(m, j.print!.started_at), from || Date.now() / 1000);
    const cursor = new Date(new Date(first * 1000).getFullYear(), new Date(first * 1000).getMonth(), 1);
    while (cursor <= now) {
      const key = monthKey(cursor.getTime() / 1000);
      months.set(key, {
        key,
        label: cursor.toLocaleDateString(locale, { month: "short" }).replace(".", ""),
        long: cursor.toLocaleDateString(locale, { month: "long", year: "numeric" }),
        prints: 0,
        failed: 0,
        secs: 0,
        grams: 0,
        cost: 0,
      });
      cursor.setMonth(cursor.getMonth() + 1);
    }
    const colours = new Map<string, { color: string; type: string; grams: number }>();
    const models = new Map<string, { title: string; count: number }>();
    let currency = "";
    for (const j of ended) {
      const p = j.print!;
      const m = months.get(monthKey(p.started_at));
      if (!m) continue;
      const part = share(j);
      if (p.outcome === "finished") m.prints++;
      else m.failed++;
      m.secs += Math.max(0, (p.ended_at ?? p.started_at) - p.started_at);
      if (j.cost) {
        m.cost += j.cost.total;
        currency ||= j.cost.currency;
      }
      for (const f of j.slice?.filaments ?? []) {
        const g = f.used_g * part;
        if (g <= 0) continue;
        m.grams += g;
        const key = `${f.color.toUpperCase()}|${f.type}`;
        const c = colours.get(key) ?? { color: f.color, type: f.type, grams: 0 };
        c.grams += g;
        colours.set(key, c);
      }
      if (p.outcome === "finished") {
        const t = models.get(j.model_title) ?? { title: j.model_title, count: 0 };
        t.count++;
        models.set(j.model_title, t);
      }
    }
    const list = [...months.values()];
    const total = list.reduce(
      (a, m) => ({ prints: a.prints + m.prints, failed: a.failed + m.failed, secs: a.secs + m.secs, grams: a.grams + m.grams, cost: a.cost + m.cost }),
      { prints: 0, failed: 0, secs: 0, grams: 0, cost: 0 },
    );
    return {
      months: list,
      total,
      currency,
      colours: [...colours.values()].sort((a, b) => b.grams - a.grams),
      models: [...models.values()].sort((a, b) => b.count - a.count).slice(0, 5),
    };
  }, [jobs, span, printerId]);

  const value = (m: Month) => (metric === "prints" ? m.prints : metric === "hours" ? m.secs / 3600 : metric === "grams" ? m.grams : m.cost);
  const show = (m: Month) =>
    metric === "prints"
      ? (m.prints === 1 ? tr("1 print") : tr("{0} prints", [m.prints])) + (m.failed ? tr(", {0} stopped", [m.failed]) : "")
      : metric === "hours"
        ? m.secs
          ? fmtDuration(m.secs)
          : "0 h"
        : metric === "grams"
          ? fmtGrams(m.grams)
          : money({ total: m.cost, currency: data.currency });
  const max = Math.max(...data.months.map(value), 0);
  const ended = data.total.prints + data.total.failed;
  const maxColour = data.colours[0]?.grams ?? 0;

  return (
    <main className="page">
      <a href="#/history" className="settings-back">
        {tr("‹ History")}
      </a>
      <h1 className="page-title">{tr("Statistics")}</h1>

      <div className="stats-filters">
        <div className="segmented" role="group" aria-label={tr("Period")}>
          {(
            [
              ["12m", tr("12 months")],
              ["year", tr("This year")],
              ["all", tr("All kept")],
            ] as const
          ).map(([k, label]) => (
            <button key={k} type="button" aria-pressed={span === k} onClick={() => setSpan(k)}>
              {label}
            </button>
          ))}
        </div>
        {printers && printers.length > 1 && (
          <select className="input" value={printerId} onChange={(e) => setPrinterId(e.target.value)} aria-label={tr("Printer")}>
            <option value="">{tr("All printers")}</option>
            {printers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        )}
      </div>

      {!jobs ? (
        <p className="faint">{tr("Loading…")}</p>
      ) : ended === 0 ? (
        <div className="empty panel">
          <p>{tr("No finished prints in this period.")}</p>
          <p className="faint">{tr("Statistics count prints that AirSlice started and followed to the end.")}</p>
        </div>
      ) : (
        <>
          <section className="stat-tiles">
            <div className="stat-tile">
              <span>{tr("Prints")}</span>
              <b className="num">{data.total.prints}</b>
              <small>{ended ? tr("{0} % finished", [Math.round((data.total.prints / ended) * 100)]) : ""}</small>
            </div>
            <div className="stat-tile">
              <span>{tr("Printing time")}</span>
              <b className="num">{fmtDuration(data.total.secs)}</b>
            </div>
            <div className="stat-tile">
              <span>{tr("Filament")}</span>
              <b className="num">{fmtGrams(data.total.grams)}</b>
            </div>
            <div className="stat-tile">
              <span>{tr("Cost")}</span>
              <b className="num">{data.total.cost > 0 ? money({ total: data.total.cost, currency: data.currency }) : "–"}</b>
              <small>{data.total.cost > 0 && data.total.prints ? tr("{0} a print", [money({ total: data.total.cost / Math.max(1, ended), currency: data.currency })]) : ""}</small>
            </div>
          </section>

          <section className="panel">
            <h2 style={{ margin: 0 }}>{tr("Per month")}</h2>
            <p className="faint small" style={{ margin: "2px 0 10px" }}>
              {data.months[0]?.long} – {data.months[data.months.length - 1]?.long}
            </p>
            <div>
              <div className="segmented" role="group" aria-label={tr("Show")}>
                {(
                  [
                    ["prints", tr("Prints")],
                    ["hours", tr("Time")],
                    ["grams", tr("Filament")],
                    ["cost", tr("Cost")],
                  ] as const
                ).map(([k, label]) => (
                  <button key={k} type="button" aria-pressed={metric === k} onClick={() => setMetric(k)}>
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <div className="month-chart" role="img" aria-label={tr("{0} per month", [metric])} onMouseLeave={() => setHover(null)}>
              {max > 0 && (
                <div className="month-scale">
                  <span className="num">{show({ ...data.months[0], prints: metric === "prints" ? Math.round(max) : 0, failed: 0, secs: max * 3600, grams: max, cost: max })}</span>
                </div>
              )}
              {data.months.map((m) => {
                const v = value(m);
                const h = max > 0 ? (v / max) * 100 : 0;
                return (
                  <div
                    key={m.key}
                    className={`month-col ${hover === m.key ? "hover" : ""}`}
                    onMouseEnter={() => setHover(m.key)}
                    onClick={() => setHover(hover === m.key ? null : m.key)}
                    title={`${m.long}: ${show(m)}`}
                  >
                    {hover === m.key && <div className="month-tip num">{show(m)}</div>}
                    <div className="month-bar-area">
                      <div className="month-bar" style={{ height: v > 0 ? `max(${h}%, 3px)` : 0 }} />
                    </div>
                    <span className="month-label">{m.label}</span>
                  </div>
                );
              })}
            </div>
            <details style={{ marginTop: 10 }}>
              <summary className="small muted">{tr("As a table")}</summary>
              <table className="stats-table num">
                <thead>
                  <tr>
                    <th>{tr("Month")}</th>
                    <th>{tr("Prints")}</th>
                    <th>{tr("Time")}</th>
                    <th>{tr("Filament")}</th>
                    <th>{tr("Cost")}</th>
                  </tr>
                </thead>
                <tbody>
                  {[...data.months].reverse().map((m) => (
                    <tr key={m.key}>
                      <td>{m.long}</td>
                      <td>
                        {m.prints}
                        {m.failed ? tr(" (+{0} stopped)", [m.failed]) : ""}
                      </td>
                      <td>{m.secs ? fmtDuration(m.secs) : "–"}</td>
                      <td>{m.grams ? fmtGrams(m.grams) : "–"}</td>
                      <td>{m.cost ? money({ total: m.cost, currency: data.currency }) : "–"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </details>
          </section>

          {data.colours.length > 0 && (
            <section className="panel">
              <h2 style={{ marginTop: 0 }}>{tr("Filament by colour")}</h2>
              <ul className="colour-bars">
                {data.colours.slice(0, 10).map((c) => (
                  <li key={`${c.color}${c.type}`}>
                    <span className="swatch" style={{ background: c.color }} aria-hidden />
                    <span className="colour-name">{c.type || tr("Filament")}</span>
                    <span className="colour-track">
                      <span className="colour-fill" style={{ width: `${(c.grams / maxColour) * 100}%` }} />
                    </span>
                    <span className="num colour-value">{fmtGrams(c.grams)}</span>
                  </li>
                ))}
              </ul>
              {data.colours.length > 10 && <p className="faint small">{tr("And {0} more colours.", [data.colours.length - 10])}</p>}
            </section>
          )}

          {data.models.length > 0 && (
            <section className="panel">
              <h2 style={{ marginTop: 0 }}>{tr("Printed most")}</h2>
              <ol className="top-models">
                {data.models.map((m) => (
                  <li key={m.title}>
                    <span className="ellipsis">{m.title}</span>
                    <b className="num">{m.count}×</b>
                  </li>
                ))}
              </ol>
            </section>
          )}
        </>
      )}
    </main>
  );
}
