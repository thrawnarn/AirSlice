import { useMemo, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { api } from "./api";
import { money } from "./Trays";
import type { Job, Printer } from "./types";
import { fmtDuration, timeAgo, toast, usePoll, remoteImg } from "./ui";
import { locale, tr } from "./i18n";

const SWIPE_REMOVE_AT = 1 / 3;

const LABEL: Record<string, string> = {
  downloading: tr("Downloading"),
  ready: tr("Waiting for plate choice"),
  slicing: tr("Slicing"),
  sliced: tr("Ready to print"),
  queued: tr("Queued"),
  waiting: tr("Waiting for the printer"),
  uploading: tr("Sending"),
  starting: tr("Starting"),
  started: tr("Printing"),
  finished: tr("Printed"),
  failed: tr("Failed"),
};

/** Totals over finished prints: count, printing time, filament and cost. */
function Stats({ jobs }: { jobs: Job[] }) {
  const [span, setSpan] = useState<"month" | "all">("month");
  const t = useMemo(() => {
    const monthStart = new Date();
    monthStart.setDate(1);
    monthStart.setHours(0, 0, 0, 0);
    const from = span === "month" ? monthStart.getTime() / 1000 : 0;
    let prints = 0, failed = 0, secs = 0, grams = 0, cost = 0;
    let currency = "";
    for (const j of jobs) {
      const p = j.print;
      if (!p?.ended_at || p.started_at < from) continue;
      if (p.outcome === "finished") {
        prints++;
        grams += j.slice?.weight_g ?? 0;
      } else if (p.outcome === "failed" || p.outcome === "cancelled") failed++;
      else continue;
      secs += Math.max(0, p.ended_at - p.started_at);
      if (j.cost) {
        cost += j.cost.total;
        currency ||= j.cost.currency;
      }
    }
    return { prints, failed, secs, grams, cost, currency };
  }, [jobs, span]);
  return (
    <section className="panel history-stats">
      <div className="row">
        <h2 className="grow small" style={{ margin: 0 }}>
          {span === "month" ? new Date().toLocaleDateString(locale, { month: "long" }).replace(/^./, (c) => c.toUpperCase()) : tr("All kept jobs")}
        </h2>
        <div className="segmented" role="group" aria-label={tr("Period")}>
          <button type="button" aria-pressed={span === "month"} onClick={() => setSpan("month")}>
            {tr("This month")}
          </button>
          <button type="button" aria-pressed={span === "all"} onClick={() => setSpan("all")}>
            {tr("All")}
          </button>
        </div>
      </div>
      <div className="temps">
        <div>
          <span>{tr("Prints")}</span>
          <b className="num">{t.prints}</b>
          {t.failed > 0 && <span className="faint">{tr("{0} stopped or failed", [t.failed])}</span>}
        </div>
        <div>
          <span>{tr("Printing")}</span>
          <b className="num">{t.secs ? fmtDuration(t.secs) : "–"}</b>
        </div>
        <div>
          <span>{tr("Filament")}</span>
          <b className="num">{t.grams ? (t.grams >= 1000 ? `${(t.grams / 1000).toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })} kg` : `${Math.round(t.grams)} g`) : "–"}</b>
        </div>
        {t.cost > 0 && (
          <div>
            <span>{tr("Cost")}</span>
            <b className="num">{money({ total: t.cost, currency: t.currency })}</b>
          </div>
        )}
      </div>
      <a href="#/history/stats" className="small">
        {tr("More statistics ›")}
      </a>
    </section>
  );
}

/** A history row: on touch screens, swiping it to the left past a third of its width removes it. */
function SwipeRow({ enabled, onRemove, children }: { enabled: boolean; onRemove: () => Promise<void>; children: ReactNode }) {
  const [dx, setDx] = useState(0);
  const [dragging, setDragging] = useState(false);
  // What's behind the row stays until the row has slid back over it.
  const [showBehind, setShowBehind] = useState(false);
  const drag = useRef<{ id: number; x: number; y: number; moved: boolean } | null>(null);
  const width = useRef(0);
  // The browser still sends a click when the finger lifts; a swipe mustn't also open the job.
  const swiped = useRef(false);

  function down(e: PointerEvent<HTMLLIElement>) {
    // A second finger neither starts a swipe nor steers the one under way.
    if (!e.isPrimary) return;
    swiped.current = false;
    if (!enabled || e.pointerType !== "touch") return;
    drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false };
    width.current = e.currentTarget.offsetWidth;
  }
  function move(e: PointerEvent<HTMLLIElement>) {
    const d = drag.current;
    if (d?.id !== e.pointerId) return;
    const mx = d.x - e.clientX; // distance to the left
    const my = e.clientY - d.y;
    if (!d.moved) {
      if (Math.abs(mx) < 10 && Math.abs(my) < 10) return;
      if (mx <= 0 || Math.abs(my) >= Math.abs(mx)) {
        drag.current = null;
        return;
      }
      d.moved = swiped.current = true;
      e.currentTarget.setPointerCapture(e.pointerId);
      setDragging(true);
      setShowBehind(true);
    }
    setDx(Math.max(0, mx));
  }
  async function up(e: PointerEvent<HTMLLIElement>) {
    const d = drag.current;
    if (d?.id !== e.pointerId) return;
    drag.current = null;
    if (!d.moved) return;
    setDragging(false);
    if (d.x - e.clientX < width.current * SWIPE_REMOVE_AT) return setDx(0);
    setDx(width.current);
    await onRemove();
    setDx(0); // still here only if removing failed
  }
  function cancel(e: PointerEvent<HTMLLIElement>) {
    if (drag.current?.id !== e.pointerId) return;
    drag.current = null;
    setDragging(false);
    setDx(0);
  }

  return (
    <li
      className="swipe-row"
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      onClickCapture={(e) => {
        if (!swiped.current) return;
        swiped.current = false;
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {showBehind && (
        <div className={`swipe-behind${dx >= width.current * SWIPE_REMOVE_AT ? " armed" : ""}`} aria-hidden="true">
          {tr("Remove")}
        </div>
      )}
      <div
        className={`swipe-content${dragging ? " dragging" : ""}`}
        style={dx > 0 ? { transform: `translateX(${-dx}px)` } : undefined}
        onTransitionEnd={(e) => e.target === e.currentTarget && dx === 0 && setShowBehind(false)}
      >
        {children}
      </div>
    </li>
  );
}

export function HistoryView({ printers, openJob }: { printers: Printer[] | null; openJob: (id: string) => void }) {
  const [jobs, setJobs] = useState<Job[] | null>(null);
  const [filter, setFilter] = useState<"all" | "favorites">(() => (localStorage.getItem("history-filter") === "favorites" ? "favorites" : "all"));
  const choose = (f: "all" | "favorites") => {
    setFilter(f);
    try {
      localStorage.setItem("history-filter", f);
    } catch {
      /* private mode */
    }
  };
  const [query, setQuery] = useState("");
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const matches = (j: Job) => {
    const text = `${j.model_title} ${j.file.name} ${j.plate_label ?? ""}`.toLowerCase();
    return words.every((w) => text.includes(w));
  };
  usePoll(async () => {
    try {
      setJobs(await api.get<Job[]>("jobs?limit=5000"));
    } catch {
      /* keep last list */
    }
  }, 5000);

  async function toggleFavorite(job: Job) {
    try {
      const updated = await api.post<Job>(`jobs/${job.id}/favorite`, { favorite: !job.favorite });
      setJobs((list) => list?.map((j) => (j.id === job.id ? { ...j, favorite: updated.favorite } : j)) ?? null);
    } catch (e) {
      toast((e as Error).message);
    }
  }

  async function remove(job: Job) {
    try {
      await api.del(`jobs/${job.id}`);
      setJobs((j) => j?.filter((x) => x.id !== job.id) ?? null);
    } catch (e) {
      toast((e as Error).message);
    }
  }

  const removable = (j: Job) => !["downloading", "slicing", "uploading", "starting", "waiting", "started"].includes(j.state) && !j.favorite;

  return (
    <main className="page">
      <div className="row">
        <h1 className="page-title grow">{tr("History")}</h1>
        <div className="segmented" role="group" aria-label={tr("Show")}>
          <button type="button" aria-pressed={filter === "all"} onClick={() => choose("all")}>
            {tr("All")}
          </button>
          <button type="button" aria-pressed={filter === "favorites"} onClick={() => choose("favorites")}>
            {tr("Favourites")}
          </button>
        </div>
      </div>
      {jobs && jobs.some((j) => j.print?.ended_at) && <Stats jobs={jobs} />}
      {jobs && jobs.length > 3 && (
        <input
          className="input history-search"
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={tr("Search by name")}
          aria-label={tr("Search the history")}
          autoCapitalize="off"
        />
      )}
      {jobs && words.length > 0 && !jobs.some((j) => (filter === "all" || j.favorite) && matches(j)) && (
        <p className="faint" style={{ textAlign: "center" }}>
          {tr("Nothing matches “{0}”.", [query])}
        </p>
      )}
      {jobs && filter === "favorites" && jobs.every((j) => !j.favorite) && (
        <div className="empty panel">
          <p>{tr("No favourites yet.")}</p>
          <p className="faint">{tr("Tap the star on a job to keep it here, ready to print again.")}</p>
        </div>
      )}
      {jobs && jobs.length === 0 && filter === "all" && (
        <div className="empty panel">
          <p>{tr("Nothing printed yet.")}</p>
          <a className="btn primary" href="#/">
            {tr("Print a model")}
          </a>
        </div>
      )}
      <ul className="files">
        {jobs?.filter((j) => (filter === "all" || j.favorite) && matches(j)).map((j) => {
          const printer = printers?.find((p) => p.id === (j.print?.printer_id ?? j.slice?.printer_id));
          const bits = [LABEL[j.state] ?? j.state];
          if (j.plate_label) bits.unshift(j.plate_label[0].toUpperCase() + j.plate_label.slice(1));
          if (printer) bits.push(printer.name);
          if (j.slice?.prediction_secs) bits.push(fmtDuration(j.slice.prediction_secs));
          if (j.cost) bits.push(money(j.cost));
          if (j.files_removed && j.source !== "external") bits.push(tr("files removed"));
          bits.push(timeAgo(j.created_at));
          return (
            <SwipeRow key={j.id} enabled={removable(j)} onRemove={() => remove(j)}>
              <button className="file" onClick={() => openJob(j.id)} type="button">
                {/* The model's own picture first: the camera's photo of the result is often dark and blurry. */}
                {j.model_cover ? (
                  <img src={remoteImg(j.model_cover)} alt="" loading="lazy" />
                ) : j.inspection?.plates.find((p) => p.has_thumbnail) ? (
                  <img
                    className="plate-cover"
                    src={`api/jobs/${j.id}/plates/${(j.slice && j.inspection.plates.find((p) => p.index === j.slice!.plate && p.has_thumbnail)?.index) || j.inspection.plates.find((p) => p.has_thumbnail)!.index}/thumbnail`}
                    alt=""
                    loading="lazy"
                  />
                ) : j.photo ? (
                  <img src={`api/jobs/${j.id}/photo.jpg`} alt="" loading="lazy" />
                ) : (
                  <div className="thumb-ph">{j.source === "external" ? "↗" : j.file.kind.toUpperCase()}</div>
                )}
                <div className="history-text">
                  <div className="file-name">{j.model_title}</div>
                  {j.source === "external" && <span className="tag external">{tr("Started outside AirSlice")}</span>}
                  <div className="file-meta">{bits.join(", ")}</div>
                  {j.state === "failed" && j.error && (
                    <div className="faint ellipsis" style={{ color: "var(--err)" }}>
                      {j.error}
                    </div>
                  )}
                </div>
              </button>
              <div className="history-actions">
                <button
                  className="btn ghost small star"
                  aria-pressed={j.favorite}
                  onClick={() => toggleFavorite(j)}
                  aria-label={j.favorite ? tr("Remove {0} from favourites", [j.model_title]) : tr("Save {0} as favourite", [j.model_title])}
                >
                  {j.favorite ? "★" : "☆"}
                </button>
                {removable(j) && (
                  <button
                    className="btn ghost small history-remove"
                    onClick={() => window.confirm(tr("Remove “{0}” from the history? Its files are deleted too.", [j.model_title])) && remove(j)}
                    aria-label={tr("Remove {0} from history", [j.model_title])}
                  >
                    ✕
                  </button>
                )}
              </div>
            </SwipeRow>
          );
        })}
      </ul>
    </main>
  );
}
