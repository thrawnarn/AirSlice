import { Suspense, lazy, useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { api, findModelLink } from "./api";
import type { ConnectionSettings, FilamentInfo, Job, MeshTransform, NextPlateSuggestion, PlateInfo, SliceTweaks, Printer, ProfileChoices, QueueEntry, ResolvedModel, Spool as SpoolmanSpool, Suggestion, Tray, TrayFilament } from "./types";
import { CollectionPicker } from "./CollectionsView";
import { PrintControls } from "./PrintersView";
import { TrayPicker, TraysSheet, loadedTrays, money, plateName, spoolLine, usePlates, usePrinterSpools } from "./Trays";
import { TweakPicker, cleanTweaks, describeTweaks } from "./Tweaks";
import { CameraView, Notice, Sheet, Spool, fmtLeft, trayLabel, fmtDuration, fmtMinutes, isBusy, printerState, toast, usePoll, remoteImg } from "./ui";
import { locale, tr } from "./i18n";
import { ModelPanel, SOURCE_NAMES, shortModel } from "./ModelView";
import { QueueBoard } from "./Queue";

const PlatePreview = lazy(() => import("./PlatePreview"));
const LayerView = lazy(() => import("./LayerView"));


function Preview(props: Parameters<typeof PlatePreview>[0]) {
  return (
    <Suspense
      fallback={
        <div className="preview">
          <div className="preview-canvas">
            <div className="preview-msg">{tr("Loading preview…")}</div>
          </div>
        </div>
      }
    >
      <PlatePreview {...props} />
    </Suspense>
  );
}

type Props = {
  jobId: string | null;
  printers: Printer[] | null;
  sharedLink: string | null;
  onSharedConsumed: () => void;
  openJob: (id: string) => void;
  reset: () => void;
};

export function PrintView(props: Props) {
  const def = props.printers?.find((p) => p.is_default) ?? props.printers?.[0] ?? null;
  return (
    <main className="page">
      {def ? <StatusStrip printer={def} /> : props.printers && <NoPrinter />}
      {props.jobId ? (
        <JobFlow id={props.jobId} printers={props.printers ?? []} reset={props.reset} openJob={props.openJob} />
      ) : (
        <Intake {...props} defaultPrinter={def} />
      )}
    </main>
  );
}

function NoPrinter() {
  return (
    <Notice tone="warn">
      {tr("No printer yet.")} <a href="#/settings/printers/new">{tr("Add your printer")}</a> {tr("to start printing.")}
    </Notice>
  );
}

export function StatusStrip({ printer }: { printer: Printer }) {
  const s = printer.status;
  const st = printerState(s, printer);
  const busy = isBusy(s);
  return (
    <a className="strip" href="#/printers" aria-label={`${printer.name}: ${st.text}`}>
      <div className="row">
        <span className={`dot ${st.tone}`} />
        <span className="strip-name grow ellipsis">{printer.name}</span>
        <span className="strip-state num">
          {busy && s.percent != null ? tr("{0}%, {1} left", [s.percent, fmtMinutes(s.remaining_minutes)]) : st.text}
          {printer.queue_length > 0 ? tr(", {0} queued", [printer.queue_length]) : ""}
        </span>
      </div>
      {busy && (
        <>
          {s.task_name && <div className="strip-state ellipsis" style={{ marginTop: 2 }}>{s.task_name}</div>}
          <div className="strip-track">
            <div className="strip-fill" style={{ width: `${Math.min(100, s.percent ?? 0)}%` }} />
          </div>
        </>
      )}
    </a>
  );
}

// ------------------------------------------------------------------ intake


function Intake({ sharedLink, onSharedConsumed, openJob, defaultPrinter, printers }: Props & { defaultPrinter: Printer | null }) {
  const [link, setLink] = useState("");
  const [model, setModel] = useState<ResolvedModel | null>(null);
  const [busy, setBusy] = useState<"" | "resolve" | "upload">("");
  const [error, setError] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  async function resolve(url: string) {
    setError(null);
    setModel(null);
    setBusy("resolve");
    try {
      const m = await api.post<ResolvedModel>("resolve", { url });
      setModel(m);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy("");
    }
  }

  useEffect(() => {
    if (sharedLink) {
      setLink(sharedLink);
      onSharedConsumed();
      resolve(sharedLink);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sharedLink]);

  function submit(e: FormEvent) {
    e.preventDefault();
    const url = findModelLink(link) ?? link.trim();
    if (url) resolve(url);
  }

  async function pasteAndGo() {
    try {
      const text = await navigator.clipboard.readText();
      const url = findModelLink(text);
      if (!url) {
        setError(tr("The clipboard doesn't contain a MakerWorld, Printables or Thingiverse link."));
        return;
      }
      setLink(url);
      resolve(url);
    } catch {
      setError(tr("Couldn't read the clipboard. Paste the link into the field instead."));
    }
  }

  async function upload(file: File) {
    setBusy("upload");
    setError(null);
    try {
      const form = new FormData();
      form.append("file", file);
      const job = await api.upload<Job>("jobs/upload", form);
      openJob(job.id);
    } catch (e) {
      setError((e as Error).message);
      setBusy("");
    }
  }

  const canPaste = typeof navigator !== "undefined" && !!navigator.clipboard?.readText;

  return (
    <>
      <h1 className="page-title">{tr("What are we printing?")}</h1>
      {!model && <NextPlateHint openJob={openJob} />}
      <form className="intake" onSubmit={submit}>
        <label className="field" style={{ marginBottom: 10 }}>
          <span>{tr("Model link from MakerWorld, Printables or Thingiverse")}</span>
          <div className="intake-row">
            <input
              className="input grow"
              type="url"
              inputMode="url"
              placeholder={tr("https://makerworld.com/en/models/…")}
              value={link}
              onChange={(e) => setLink(e.target.value)}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
        </label>
        <div className="row">
          <button className="btn primary grow" disabled={!link.trim() || busy !== ""}>
            {busy === "resolve" ? tr("Looking up…") : tr("Find files")}
          </button>
          {canPaste && (
            <button type="button" className="btn" onClick={pasteAndGo} disabled={busy !== ""}>
              {tr("Paste link")}
            </button>
          )}
        </div>
        <a className="intake-collections" href="#/explore">
          {tr("Browse your MakerWorld collections ›")}
        </a>
        <div className="intake-alt faint">
          <span>{tr("Have the file already?")}</span>
          <button type="button" className="btn ghost small" onClick={() => fileInput.current?.click()} disabled={busy !== ""}>
            {busy === "upload" ? tr("Uploading…") : tr("Upload 3MF or STL")}
          </button>
          <input
            ref={fileInput}
            type="file"
            accept=".3mf,.stl,.obj,.step,.stp"
            hidden
            onChange={(e) => e.target.files?.[0] && upload(e.target.files[0])}
          />
        </div>
      </form>

      {error && <Notice tone="err">{error}</Notice>}

      {model && <ModelPanel key={model.url} model={model} defaultPrinter={defaultPrinter} openJob={openJob} />}
      {!model && <QueueBoard printers={printers} />}
    </>
  );
}


// ------------------------------------------------------------------ job

const STEPS: { key: string; label: string; states: string[] }[] = [
  { key: "download", label: tr("Download"), states: ["downloading"] },
  { key: "choose", label: tr("Choose plate"), states: ["ready"] },
  { key: "slice", label: tr("Slice"), states: ["slicing"] },
  { key: "check", label: tr("Check filaments"), states: ["sliced"] },
  { key: "send", label: tr("Send to printer"), states: ["queued", "waiting", "uploading", "starting"] },
  { key: "done", label: tr("Printing"), states: ["started", "finished"] },
];

function stepIndex(job: Job): number {
  if (job.state === "failed") {
    if (job.slice || job.source === "external") return 3;
    if (job.inspection) return 1;
    return 0;
  }
  return STEPS.findIndex((s) => s.states.includes(job.state));
}

function JobFlow({ id, printers, reset, openJob }: { id: string; printers: Printer[]; reset: () => void; openJob: (id: string) => void }) {
  const [job, setJob] = useState<Job | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = !!job && ["downloading", "slicing", "uploading", "starting", "waiting"].includes(job.state);

  async function load() {
    try {
      setJob(await api.get<Job>(`jobs/${id}`));
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    }
  }
  usePoll(load, busy || !job ? 1200 : 5000);

  if (!job) {
    return error ? <Notice tone="err">{error}</Notice> : <p className="muted">{tr("Loading…")}</p>;
  }
  const current = stepIndex(job);

  return (
    <>
      <div className="model-head">
        {job.model_cover ? (
          <img src={remoteImg(job.model_cover)} alt="" />
        ) : job.inspection?.plates.find((p) => p.has_thumbnail) ? (
          <img className="plate-cover" src={`api/jobs/${job.id}/plates/${job.inspection.plates.find((p) => p.has_thumbnail)!.index}/thumbnail`} alt="" />
        ) : (
          <div className="cover-ph" />
        )}
        <div style={{ minWidth: 0 }}>
          <p className="model-title">{job.model_title}</p>
          <p className="faint" style={{ margin: 0 }}>
            {job.plate_label ? `${job.plate_label[0].toUpperCase()}${job.plate_label.slice(1)}, ` : ""}
            {job.file.name}
          </p>
          {job.source !== "upload" && job.model_url && (
            <p className="model-links small">
              <a href={`#/explore/model?u=${encodeURIComponent(job.model_url)}`}>
                {job.source === "makerworld" ? tr("Other print profiles") : tr("Other files")}
              </a>
              <a href={job.model_url} target="_blank" rel="noreferrer">
                {tr("Open on {0} ↗", [SOURCE_NAMES[job.source] ?? job.source])}
              </a>
            </p>
          )}
          {job.inspection && (
            <button
              type="button"
              className="btn ghost small star"
              aria-pressed={job.favorite}
              onClick={async () => {
                try {
                  setJob(await api.post<Job>(`jobs/${job.id}/favorite`, { favorite: !job.favorite }));
                  toast(job.favorite ? tr("Removed from favourites") : tr("Saved to favourites"));
                } catch (e) {
                  toast((e as Error).message);
                }
              }}
            >
              {job.favorite ? tr("★ Favourite") : tr("☆ Save as favourite")}
            </button>
          )}
        </div>
      </div>

      <div className="progress" role="progressbar" aria-valuemin={1} aria-valuemax={STEPS.length} aria-valuenow={current + 1} aria-valuetext={STEPS[current]?.label}>
        <ol>
          {STEPS.map((s, i) => (
            <li key={s.key} className={i < current || job.state === "finished" ? "done" : i === current ? (job.state === "failed" ? "bad" : "now") : ""} />
          ))}
        </ol>
        <p className="row small">
          <span className="grow" style={{ fontWeight: 620 }}>
            {job.state === "failed"
              ? tr("{0} failed", [STEPS[current]?.label])
              : job.state === "queued"
                ? tr("Queued")
                : job.state === "waiting"
                  ? tr("Waiting for the printer")
                  : job.state === "finished"
                    ? job.print?.outcome === "unknown"
                      ? tr("Sent to the printer")
                      : tr("Printed")
                    : STEPS[current]?.label}
          </span>
          <span className="faint num">
            {tr("Step {0} of {1}", [current + 1, STEPS.length])}
          </span>
        </p>
      </div>

      {job.source === "external" && (
        <Notice>{tr("Started outside AirSlice, on the printer or from another app. AirSlice keeps it in the history but has no file to print it again, and it only knows the filament used if a log said so.")}</Notice>
      )}
      {job.state === "failed" && job.error && <Notice tone="err">{job.error}</Notice>}
      {error && <Notice tone="err">{error}</Notice>}

      {job.source !== "external" && job.files_removed && !["started", "finished"].includes(job.state) && (
        <>
          <Notice>{tr("The files of this job were removed to save space.")}</Notice>
          <button className="btn go" onClick={() => (job.source === "upload" ? reset() : reprint(job.id, openJob))}>
            {job.source === "upload" ? tr("Upload it again") : tr("Download and print again")}
          </button>
        </>
      )}
      {!job.files_removed && (job.state === "ready" || (job.state === "failed" && job.inspection && !job.slice)) && (
        <SliceOptions job={job} printers={printers} onChange={setJob} />
      )}
      {!job.files_removed && (job.state === "sliced" || (job.state === "failed" && job.slice)) && (
        <PrintOptions job={job} printers={printers} onChange={setJob} />
      )}
      {job.state === "queued" && <Queued job={job} printers={printers} onChange={setJob} />}
      {job.state === "waiting" && <Waiting job={job} printers={printers} onChange={setJob} />}
      {(job.state === "started" || job.state === "finished") && <Started job={job} printers={printers} reset={reset} openJob={openJob} onChange={setJob} />}
      {job.next_plate && <QueueNextPlate job={job} plate={job.next_plate} openJob={openJob} />}
      {job.state === "failed" && job.inspection && (
        <button className="btn block" style={{ marginTop: 12 }} onClick={() => reprint(job.id, openJob)}>
          {tr("Print again from the start")}
        </button>
      )}
      {busy && <p className="muted" style={{ marginTop: 12 }}>{job.log[job.log.length - 1]}</p>}
      {(job.state === "uploading" || job.state === "starting") && (
        <Notice>{tr("You don't have to wait here. Sending carries on in the background, and you can close the app or print something else.")}</Notice>
      )}
      {job.state === "uploading" && (
        <div className="slice-progress" role="progressbar" aria-label={tr("Sending")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={job.progress?.percent ?? 0}>
          <div className="row small">
            <span className="grow">{job.progress?.message || tr("Connecting to the printer…")}</span>
            <b className="num">{job.progress ? `${job.progress.percent}%` : ""}</b>
          </div>
          <div className="strip-track">
            <div className="strip-fill" style={{ width: `${job.progress?.percent ?? 2}%` }} />
          </div>
        </div>
      )}
      {job.state === "starting" && (
        <div className="slice-progress" role="progressbar" aria-label={tr("Starting")} aria-busy="true">
          <div className="row small">
            <span className="grow">{tr("Waiting for the printer to start…")}</span>
          </div>
          <div className="strip-track">
            <div className="strip-fill indeterminate" />
          </div>
        </div>
      )}
      {job.state === "slicing" && (
        <div className="slice-progress" role="progressbar" aria-label={tr("Slicing")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={job.progress?.percent ?? 0}>
          <div className="row small">
            <span className="grow">{job.progress?.message || tr("Starting the slicer…")}</span>
            <b className="num">{job.progress ? `${job.progress.percent}%` : ""}</b>
          </div>
          <div className="strip-track">
            <div className="strip-fill" style={{ width: `${job.progress?.percent ?? 2}%` }} />
          </div>
        </div>
      )}
      {job.state === "failed" && !job.inspection && job.source !== "external" && (
        <button className="btn block" onClick={reset} style={{ marginTop: 12 }}>
          {tr("Start over")}
        </button>
      )}

      <details style={{ marginTop: 24 }}>
        <summary className="faint">{tr("Details")}</summary>
        <ul className="log">
          {job.log.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      </details>
    </>
  );
}

function PrinterPicker({ printers, value, onChange }: { printers: Printer[]; value: string; onChange: (id: string) => void }) {
  if (printers.length <= 1) return null;
  return (
    <label className="field">
      <span>{tr("Printer")}</span>
      <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
        {printers.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name} ({printerState(p.status).text.toLowerCase()})
          </option>
        ))}
      </select>
    </label>
  );
}

/** What's chosen for one added model. */
type ExtraPick = { copies: number; tray: number; plate?: number };

/** Jobs whose model can go on another plate, newest first, one per file. */
function addableJobs(jobs: Job[], query: string): Job[] {
  const seen = new Set<string>();
  const q = query.trim().toLowerCase();
  return jobs.filter((j) => {
    if (j.source === "external" || (j.source === "upload" && (j.files_removed || !j.inspection))) return false;
    if (q && !j.model_title.toLowerCase().includes(q)) return false;
    const key = `${j.source}|${j.model_url}|${j.file.id}|${j.file.name}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** Choose a model to add: from a link or file, MakerWorld collections, or earlier jobs. */
function AddModelSheet({ job, onAdded, onClose }: { job: Job; onAdded: (j: Job) => void; onClose: () => void }) {
  const [tab, setTab] = useState<"link" | "collections" | "history">("link");
  const [link, setLink] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<Job[] | null>(null);
  const [query, setQuery] = useState("");
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (tab !== "history" || jobs) return;
    api.get<Job[]>("jobs?limit=5000").then(setJobs, (e) => setError((e as Error).message));
  }, [tab, jobs]);

  async function add(work: () => Promise<Job>) {
    setBusy(true);
    setError(null);
    try {
      onAdded(await work());
      onClose();
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }
  const addUrl = (url: string) => add(() => api.post<Job>(`jobs/${job.id}/extras`, { url }));
  const addFile = (file: File) => {
    const form = new FormData();
    form.append("file", file);
    void add(() => api.upload<Job>(`jobs/${job.id}/extras/upload`, form));
  };
  const tabs: ["link" | "collections" | "history", string][] = [
    ["link", tr("Link")],
    ["collections", tr("Collections")],
    ["history", tr("History")],
  ];
  const found = jobs ? addableJobs(jobs, query).slice(0, 100) : [];

  return (
    <Sheet title={tr("Add a model")} onClose={onClose}>
      <div className="segmented" role="group" aria-label={tr("Add a model")} style={{ marginBottom: 14 }}>
        {tabs.map(([id, label]) => (
          <button key={id} type="button" aria-pressed={tab === id} onClick={() => setTab(id)}>
            {label}
          </button>
        ))}
      </div>
      {busy && <Notice>{tr("Adding the model. A large file can take a while…")}</Notice>}
      {error && <Notice tone="err">{error}</Notice>}

      {tab === "link" && (
        <>
          <form
            className="row"
            style={{ gap: 8 }}
            onSubmit={(e) => {
              e.preventDefault();
              if (link.trim()) void addUrl(link.trim());
            }}
          >
            <input className="input grow" value={link} onChange={(e) => setLink(e.target.value)} placeholder={tr("Paste a model link")} aria-label={tr("Link to the model to add")} disabled={busy} autoFocus />
            <button className="btn small" type="submit" disabled={busy || !link.trim()}>
              {tr("Add")}
            </button>
          </form>
          <p className="faint small" style={{ margin: "12px 0 0" }}>
            {tr("Or from this device:")}{" "}
            <button type="button" className="linkish" disabled={busy} onClick={() => fileInput.current?.click()}>
              {tr("Choose file")}
            </button>
          </p>
          <input
            ref={fileInput}
            type="file"
            accept=".3mf,.stl,.obj,.step,.stp"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              if (f) addFile(f);
            }}
          />
        </>
      )}

      {tab === "collections" && <CollectionPicker disabled={busy} onPick={(url) => void addUrl(url)} />}

      {tab === "history" && (
        <>
          <input className="input" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={tr("Search earlier models")} aria-label={tr("Search earlier models")} style={{ width: "100%", marginBottom: 10 }} />
          {!jobs && !error && <p className="faint">{tr("Loading…")}</p>}
          {jobs && found.length === 0 && <p className="faint">{tr("No earlier models to add.")}</p>}
          <ul className="files">
            {found.map((j) => (
              <li key={j.id}>
                <button type="button" className="file" disabled={busy} onClick={() => void add(() => api.post<Job>(`jobs/${job.id}/extras/from-job`, { job_id: j.id }))}>
                  {j.model_cover ? (
                    <img src={remoteImg(j.model_cover)} alt="" loading="lazy" />
                  ) : j.inspection?.plates.find((p) => p.has_thumbnail) ? (
                    <img className="plate-cover" src={`api/jobs/${j.id}/plates/${j.inspection.plates.find((p) => p.has_thumbnail)!.index}/thumbnail`} alt="" loading="lazy" />
                  ) : (
                    <div className="thumb-ph">{j.file.kind.toUpperCase()}</div>
                  )}
                  <div className="history-text">
                    <div className="file-name ellipsis">{j.model_title}</div>
                    <div className="file-meta ellipsis">{j.file.name}</div>
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </Sheet>
  );
}

/** Other models to print on the same plate. */
function PlateExtras({
  job,
  picks,
  trays,
  onPick,
  onChange,
  onManage,
  spools,
}: {
  job: Job;
  picks: Record<string, ExtraPick>;
  trays: Tray[];
  onPick: (id: string, pick: Partial<ExtraPick>) => void;
  onChange: (j: Job) => void;
  onManage: () => void;
  spools?: Record<string, SpoolmanSpool> | null;
}) {
  const extras = job.extras ?? [];
  const [adding, setAdding] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function remove(id: string) {
    setBusy(true);
    setError(null);
    try {
      onChange(await api.del<Job>(`jobs/${job.id}/extras/${id}`));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ margin: "4px 0 14px" }}>
      {extras.length > 0 && (
        <div className="panel transform" style={{ marginBottom: 8 }}>
          {extras.map((x) => {
            const pick = picks[x.id] ?? { copies: 1, tray: -1 };
            return (
              <div key={x.id} className="extra">
                <div className="row" style={{ gap: 10 }}>
                  {x.cover && <img className="extra-cover" src={remoteImg(x.cover)} alt="" loading="lazy" />}
                  <div className="grow ellipsis" style={{ fontWeight: 620 }}>{x.title}</div>
                  <button type="button" className="btn small" disabled={busy} onClick={() => void remove(x.id)}>
                    {tr("Remove")}
                  </button>
                </div>
                {x.multicolor && <p className="faint small" style={{ margin: "6px 0 0" }}>{tr("This model has several colours in its file. Added to this plate, it prints in one colour.")}</p>}
                {x.plates.length > 1 && (
                  <label className="field" style={{ margin: "8px 0 0" }}>
                    <span>{tr("Plate from this file")}</span>
                    <select className="input" value={pick.plate ?? x.plates[0].index} onChange={(e) => onPick(x.id, { plate: Number(e.target.value) })}>
                      {x.plates.map((p) => (
                        <option key={p.index} value={p.index}>
                          {p.name ?? tr("Plate {0}", [p.index])}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                {trays.length > 0 && (
                  <div className="tf-row">
                    <span className="tf-label">{tr("Print with")}</span>
                    <TrayPicker title={tr("Print {0} with", [x.title])} trays={trays} value={pick.tray} onChange={(id) => onPick(x.id, { tray: id })} onManage={onManage} spools={spools} />
                  </div>
                )}
                <CopiesStepper value={pick.copies} onChange={(n) => onPick(x.id, { copies: n })} />
              </div>
            );
          })}
          <p className="faint small" style={{ margin: "8px 0" }}>{tr("The slicer arranges everything on the plate. You'll see the result after slicing.")}</p>
        </div>
      )}
      {error && <Notice tone="err">{error}</Notice>}
      <button type="button" className="btn block" onClick={() => setAdding(true)}>
        {tr("More models on this plate")}
      </button>
      {adding && <AddModelSheet job={job} onAdded={onChange} onClose={() => setAdding(false)} />}
    </div>
  );
}

function SliceOptions({ job, printers, onChange }: { job: Job; printers: Printer[]; onChange: (j: Job) => void }) {
  const insp = job.inspection!;
  const initialPrinter = (printers.find((p) => p.is_default) ?? printers[0])?.id ?? "";
  const [printerId, setPrinterId] = useState(initialPrinter);
  const [plate, setPlate] = useState(insp.plates[0]?.index ?? 1);
  const [mode, setMode] = useState<"auto" | "project" | "printer">("auto");
  const [choices, setChoices] = useState<ProfileChoices | null>(null);
  const [process, setProcess] = useState("");
  const [filament, setFilament] = useState("");
  const [transform, setTransform] = useState<MeshTransform>({ scale_percent: 100, copies: 1, rotate_z: 0 });
  const [tweaks, setTweaks] = useState<SliceTweaks>(job.slice?.tweaks ?? {});
  // One tray per filament of the file (a bare mesh has one); -1 = none.
  const [trayIds, setTrayIds] = useState<number[] | null>(null);
  const [traysTouched, setTraysTouched] = useState(false);
  const [showTrays, setShowTrays] = useState(false);
  const [copies, setCopies] = useState(1);
  const plates = usePlates();
  // "" = the printer's default plate, or the file's/profile's own when it has none.
  const [bedType, setBedType] = useState("");
  // Objects of the chosen plate left out of the print.
  const [excluded, setExcluded] = useState<string[]>([]);
  useEffect(() => setExcluded([]), [plate]);
  const plateObjects = insp.plates.find((p) => p.index === plate)?.objects ?? [];
  const objectLabels = labelObjects(plateObjects);
  const [trayProfiles, setTrayProfiles] = useState<TrayFilament[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const printer = printers.find((p) => p.id === printerId) ?? null;
  const [spoolInfo] = usePrinterSpools(printer);

  useEffect(() => {
    if (!printerId && printers.length) setPrinterId(initialPrinter);
  }, [printers, printerId, initialPrinter]);

  useEffect(() => {
    if (!printer) return;
    api
      .get<ProfileChoices>(`profiles?machine=${encodeURIComponent(printer.machine_profile)}`)
      .then((c) => {
        setChoices(c);
        setProcess(printer.process_profile ?? c.default_process ?? "");
        setFilament("");
      })
      .catch(() => setChoices(null));
  }, [printer?.id, printer?.machine_profile, printer?.process_profile, printer?.filament_profile]);

  // Which slicer profile each loaded tray would use (meshes only).
  const trayKey = printer
    ? printer.status.trays.map((t) => `${t.id}${t.type}${t.color}${t.info_idx ?? ""}`).join() + (printer.status.external?.type ?? "") + (printer.status.external?.color ?? "")
    : "";
  useEffect(() => {
    if (!printer || insp.kind !== "mesh") return;
    api
      .get<TrayFilament[]>(`printers/${printer.id}/tray-filaments`)
      .then(setTrayProfiles)
      .catch(() => setTrayProfiles([]));
  }, [printer?.id, trayKey, insp.kind]);

  const sameModel = !!printer && !!insp.printer_model && shortModel(insp.printer_model) === shortModel(printer.printer_model);
  const loaded = loadedTrays(printer);
  const mesh = insp.kind === "mesh";
  const slots: FilamentInfo[] = mesh ? [{ type: "", color: "" }] : insp.filaments;

  // Suggest trays (same material, closest colour) until the user picks.
  useEffect(() => {
    if (!printer || traysTouched) return;
    if (mesh) {
      const t = loaded.find((x) => x.active) ?? loaded[0];
      setTrayIds([t ? t.id : -1]);
      return;
    }
    if (!slots.length) return;
    api
      .get<Suggestion>(`jobs/${job.id}/tray-suggestion?printer_id=${encodeURIComponent(printer.id)}`)
      // Only trays with filament in them: without AMS data the suggestion
      // falls back to the (unknown) external spool.
      .then((s) => setTrayIds(s.ams_mapping.slice(0, slots.length).map((id) => (loaded.some((t) => t.id === id) ? id : -1))))
      .catch(() => setTrayIds(slots.map(() => -1)));
  }, [printer?.id, trayKey, traysTouched, job.id]);

  const pickTray = (slot: number, tray: number) => {
    const next = [...(trayIds ?? slots.map(() => -1))];
    next[slot] = tray;
    setTrayIds(next);
    setTraysTouched(true);
  };
  const trayOf = (slot: number) => loaded.find((t) => t.id === trayIds?.[slot]) ?? null;
  const chosenTray = trayOf(0);
  const shownColor = chosenTray?.color ?? null;
  const meshColorFor = useCallback(() => shownColor, [shownColor]);
  const projectColorFor = useCallback(
    (filament: number, fileColor: string | null) => loaded.find((t) => t.id === trayIds?.[filament - 1])?.color || fileColor,
    [trayIds, trayKey],
  );
  const chosenProfile = trayProfiles.find((t) => t.tray_id === chosenTray?.id);
  const extras = job.extras ?? [];
  const [extraPicks, setExtraPicks] = useState<Record<string, ExtraPick>>(() =>
    Object.fromEntries((job.slice?.options?.extras ?? []).map((e) => [e.id, { copies: e.copies ?? 1, tray: e.tray_id ?? -1, plate: e.plate ?? undefined }])),
  );
  const extraPick = (id: string): ExtraPick => extraPicks[id] ?? { copies: 1, tray: trayIds?.find((t) => t >= 0) ?? loaded[0]?.id ?? -1 };
  const pickExtra = (id: string, pick: Partial<ExtraPick>) => setExtraPicks((p) => ({ ...p, [id]: { ...extraPick(id), ...pick } }));
  const reslice = insp.kind === "sliced" && (copies > 1 || excluded.length > 0 || extras.length > 0);
  const effective = insp.kind === "sliced" ? (reslice ? "project" : "presliced") : mesh ? "printer" : mode === "auto" ? (sameModel ? "project" : "printer") : mode;
  const anyTray = (trayIds ?? []).some((t) => t >= 0);

  useEffect(() => setBedType(printer?.default_plate ?? ""), [printer?.id, printer?.default_plate]);
  const plateChoices = printer?.plates.length ? plates.filter((p) => printer.plates.includes(p.value)) : plates;

  const sliceBody = (index: number = plate) => ({
    objects: index === plate && excluded.length ? plateObjects.filter((o) => !excluded.includes(o.id)).map((o) => o.id) : undefined,
    bed_type: bedType || undefined,
    filament_profile: mesh && filament ? filament : undefined,
    tray_id: mesh && chosenTray ? chosenTray.id : undefined,
    tray_ids: !mesh && anyTray ? trayIds : undefined,
    ...(mesh ? cleanTransform(transform) : copies > 1 && !extras.length ? { copies } : {}),
    tweaks: effective === "presliced" ? undefined : cleanTweaks(tweaks),
    extras: extras.length
      ? extras.map((x) => {
          const p = extraPick(x.id);
          return { id: x.id, copies: p.copies, plate: p.plate, tray_id: p.tray >= 0 ? p.tray : undefined };
        })
      : undefined,
  });

  async function sliceAll() {
    if (!window.confirm(tr("Slice all {0} plates and add them to the queue in order?", [insp.plates.length]))) return;
    setBusy(true);
    setError(null);
    try {
      await api.post<Job[]>(`jobs/${job.id}/slice-all`, {
        printer_id: printerId || undefined,
        settings: mode === "auto" ? undefined : mode,
        process_profile: effective === "printer" && process ? process : undefined,
        ...sliceBody(),
      });
      toast(tr("Slicing {0} plates — they'll appear in the queue", [insp.plates.length]));
      window.location.hash = "/printers";
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  const settingsBody = (index: number = plate) => ({
    printer_id: printerId || undefined,
    settings: mode === "auto" ? undefined : mode,
    process_profile: effective === "printer" && process ? process : undefined,
    ...sliceBody(index),
  });

  // Slice one plate and put it in the printer's queue; this page then shows
  // its progress until it is queued.
  async function queuePlate(index: number) {
    setPlate(index);
    setBusy(true);
    setError(null);
    try {
      const jobs = await api.post<Job[]>(`jobs/${job.id}/slice-all`, { ...settingsBody(index), plates: [index] });
      toast(insp.kind === "sliced" && copies === 1 ? tr("Added to the queue") : tr("Slicing — it goes into the queue when done"));
      if (jobs[0]) onChange(jobs[0]);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function slice(index: number = plate) {
    setPlate(index);
    setBusy(true);
    setError(null);
    try {
      const j = await api.post<Job>(`jobs/${job.id}/slice`, {
        printer_id: printerId || undefined,
        plate: index,
        settings: mode === "auto" ? undefined : mode,
        process_profile: effective === "printer" && process ? process : undefined,
        ...sliceBody(index),
      });
      onChange(j);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (!printers.length) return <NoPrinter />;

  return (
    <section>
      <PrinterPicker printers={printers} value={printerId} onChange={setPrinterId} />

      {insp.plates.length > 1 && <h2>{tr("Plate")}</h2>}
      <div className="plates" role="group" aria-label={tr("Plates")}>
        {insp.plates.map((p) => (
          <div key={p.index} className={`plate ${plate === p.index ? "chosen" : ""}`}>
            <button type="button" className="plate-pick" aria-pressed={plate === p.index} onClick={() => setPlate(p.index)}>
              <div className="plate-img">
                {p.has_thumbnail && <img src={`api/jobs/${job.id}/plates/${p.index}/thumbnail`} alt="" loading="lazy" />}
              </div>
              <div className="plate-label ellipsis">{p.name ?? tr("Plate {0}", [p.index])}</div>
              {p.object_count > 0 && (
                <div className="faint">{p.object_count === 1 ? tr("1 object") : `${p.object_count} objects`}</div>
              )}
              {(p.prediction_secs || p.weight_g) && (
                <div className="faint small num">
                  {[p.prediction_secs ? fmtDuration(p.prediction_secs) : null, p.weight_g ? `${Math.round(p.weight_g)} g` : null].filter(Boolean).join(" · ")}
                </div>
              )}
            </button>
          </div>
        ))}
      </div>

      {!mesh && plateObjects.length > 1 && (
        <details className="objects-pick" open={excluded.length > 0}>
          <summary className="small">
            {excluded.length
              ? tr("Printing {0} of {1} objects", [plateObjects.length - excluded.length, plateObjects.length])
              : tr("All {0} objects on this plate", [plateObjects.length])}
          </summary>
          <div className="check-list">
            {plateObjects.map((o, i) => (
              <label key={o.id} className="check">
                <input
                  type="checkbox"
                  checked={!excluded.includes(o.id)}
                  disabled={!excluded.includes(o.id) && excluded.length === plateObjects.length - 1}
                  onChange={(e) => setExcluded((x) => (e.target.checked ? x.filter((id) => id !== o.id) : [...x, o.id]))}
                />
                <span>{objectLabels[i]}</span>
              </label>
            ))}
          </div>
          <p className="faint small" style={{ margin: "4px 0 0" }}>
            Leave out what you don't want; the rest prints where it is on the plate.
            {insp.kind === "sliced" ? tr(" A pre-sliced file is sliced again for this.") : ""}
          </p>
        </details>
      )}

      <Preview
        jobId={job.id}
        plate={plate}
        printerId={printerId || undefined}
        source="input"
        transform={mesh ? transform : undefined}
        colorFor={mesh ? meshColorFor : projectColorFor}
        hidden={excluded}
        note={
          mesh
            ? tr("The slicer decides the final placement and may turn the model to lie flat.")
            : extras.length
              ? tr("Shows this model only. With added models, the slicer lays out the whole plate again.")
              : copies > 1
              ? tr("Shown as laid out in the file. The copies are placed when slicing.")
              : insp.kind === "project" && !sameModel
                ? tr("Shown as laid out in the file. Slicing for your printer can move things.")
                : undefined
        }
      />

      {printer && slots.length > 0 && (
        <>
          <h2>{slots.length === 1 ? tr("Filament") : tr("Filaments")}</h2>
          {loaded.length === 0 ? (
            <Notice tone="warn">
              {printer.status.connected
                ? tr("{0} isn't reporting any loaded filament. Check the AMS, or refresh the trays.", [printer.name])
                : tr("{0} is offline, so its trays aren't known. You can slice anyway and choose trays when printing.", [printer.name])}
            </Notice>
          ) : (
            <ul className="mapping panel" style={{ padding: "4px 16px" }}>
              {slots.map((f, i) => {
                const t = trayOf(i);
                return (
                  <li key={i} className="map-row">
                    <div className="row" style={{ gap: 8 }}>
                      {mesh ? (
                        <div style={{ fontWeight: 620 }}>{tr("Print with")}</div>
                      ) : (
                        <>
                          <Spool color={f.color} size={36} label={tr("Model filament {0}: {1}", [i + 1, f.type])} />
                          <div style={{ fontWeight: 620 }}>{f.type || tr("Filament {0}", [i + 1])}</div>
                        </>
                      )}
                    </div>
                    <div className="map-wire" style={{ ["--from" as string]: mesh ? t?.color || "var(--line)" : f.color, ["--to" as string]: t?.color || "var(--line)" }} />
                    <div className="map-pick">
                      <TrayPicker
                        title={mesh ? tr("Print with") : tr("Tray for filament {0} ({1})", [i + 1, f.type])}
                        trays={loaded}
                        value={trayIds?.[i] ?? -1}
                        onChange={(id) => pickTray(i, id)}
                        none={mesh ? undefined : tr("Choose later (as in the file)")}
                        onManage={() => setShowTrays(true)}
                        spools={spoolInfo?.trays}
                      />
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          <p className="faint small" style={{ margin: "8px 0 14px" }}>
            {loaded.length === 0
              ? ""
              : mesh && chosenTray
              ? filament
                ? tr("Sliced as {0}. ", [filament])
                : chosenProfile?.profile
                  ? tr("Sliced as {0}{1}. ", [chosenProfile.profile, chosenProfile.exact ? tr(" (matched by the spool)") : ""])
                  : ""
              : insp.kind === "sliced" && !reslice
                ? tr("Already sliced: the trays only decide where each filament comes from. ")
                : anyTray
                  ? tr("Sliced with each tray's filament and colour. ")
                  : ""}
            {tr("Wrong spool, or a tray changed?")}{" "}
            <button type="button" className="linkish" onClick={() => setShowTrays(true)}>
              {tr("Check the trays")}
            </button>
          </p>
          {showTrays && <TraysSheet printer={printer} onClose={() => setShowTrays(false)} />}
        </>
      )}

      {printer && (
        <PlateExtras
          job={job}
          picks={Object.fromEntries(extras.map((x) => [x.id, extraPick(x.id)]))}
          trays={loaded}
          onPick={pickExtra}
          onChange={onChange}
          onManage={() => setShowTrays(true)}
          spools={spoolInfo?.trays}
        />
      )}

      {insp.kind === "project" && printer && insp.printer_model && !sameModel && (
        <Notice tone="warn">
          {tr("This file was set up for a {0}. It will be sliced with your {1} profile instead.", [insp.printer_model.replace(/^Bambu Lab /, ""), printer.printer_model.replace(/^Bambu Lab /, "")])}
        </Notice>
      )}
      {insp.kind === "project" && printer && (
        <div className="row wrap" style={{ margin: "8px 0 12px" }}>
          <span className="small muted">{tr("Print settings")}</span>
          <div className="segmented" role="group" aria-label={tr("Print settings")}>
            <button type="button" aria-pressed={effective === "project"} onClick={() => setMode("project")}>
              {tr("From the file")}
            </button>
            <button type="button" aria-pressed={effective === "printer"} onClick={() => setMode("printer")}>
              {tr("My printer profile")}
            </button>
          </div>
        </div>
      )}
      {insp.kind === "sliced" && (
        <Notice>
          {extras.length
            ? tr("This file is already sliced. It will be sliced again together with the added models.")
            : reslice
              ? tr("This file is already sliced. It will be sliced again to fit {0} copies.", [copies])
              : tr("This file is already sliced, so it goes straight to the printer.")}
        </Notice>
      )}

      {effective !== "presliced" && plateChoices.length > 0 && (
        <label className="field">
          <span>{tr("Build plate")}</span>
          <select className="input" value={bedType} onChange={(e) => setBedType(e.target.value)}>
            {!printer?.default_plate && <option value="">{tr("As set up in the file or profile")}</option>}
            {plateChoices.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
                {p.value === printer?.default_plate ? tr(" (usually on the printer)") : ""}
              </option>
            ))}
          </select>
          {!printer?.plates.length && <small>{tr("Tell AirSlice which plates you have under Settings, Printers.")}</small>}
        </label>
      )}

      {effective === "printer" && choices && (
        <details style={{ margin: "4px 0 14px" }}>
          <summary className="small muted">{tr("Quality{0}: {1}", [insp.kind === "mesh" ? tr(" and filament") : "", process || "printer default"])}</summary>
          <div style={{ marginTop: 10 }}>
            <label className="field">
              <span>{tr("Quality")}</span>
              <select className="input" value={process} onChange={(e) => setProcess(e.target.value)}>
                {choices.processes.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            {insp.kind === "mesh" && (
              <label className="field">
                <span>{tr("Filament profile")}</span>
                <select className="input" value={filament} onChange={(e) => setFilament(e.target.value)}>
                  <option value="">{loaded.length ? tr("Match the chosen tray") : tr("Printer default")}</option>
                  {choices.filaments.map((f) => (
                    <option key={f.name} value={f.name}>
                      {f.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        </details>
      )}

      {effective !== "presliced" && <TweakPicker value={tweaks} onChange={setTweaks} />}

      {mesh ? (
        <TransformControls value={transform} onChange={setTransform} />
      ) : extras.length ? null : (
        <div className="panel transform">
          <CopiesStepper value={copies} onChange={setCopies} />
        </div>
      )}

      {error && <Notice tone="err">{error}</Notice>}
      <button className="btn go" onClick={() => slice()} disabled={busy || !printerId}>
        {insp.kind === "sliced" && !reslice ? tr("Continue") : insp.plates.length > 1 ? tr("Slice plate {0}", [plate]) : tr("Slice")}
      </button>
      <div className="row" style={{ gap: 8, marginTop: 8 }}>
        <button className="btn grow" onClick={() => queuePlate(plate)} disabled={busy || !printerId}>
          {insp.plates.length > 1 ? tr("Queue plate {0}", [plate]) : tr("Add to the queue")}
        </button>
        {insp.plates.length > 1 && !extras.length && (
          <button className="btn grow" onClick={sliceAll} disabled={busy || !printerId}>
            {tr("Queue all {0} plates", [insp.plates.length])}
          </button>
        )}
      </div>
    </section>
  );
}

function PrintOptions({ job, printers, onChange }: { job: Job; printers: Printer[]; onChange: (j: Job) => void }) {
  const slice = job.slice!;
  const printer = printers.find((p) => p.id === slice.printer_id) ?? null;
  const trays: Tray[] = job.trays ?? [];
  const [mapping, setMapping] = useState<number[] | null>(null);
  const [mappingTouched, setMappingTouched] = useState(false);
  const [opts, setOpts] = useState({ bed_leveling: true, flow_cali: true, timelapse: false });
  const [showTrays, setShowTrays] = useState(false);
  const plates = usePlates();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Take the suggestion once per slice; afterwards the user's picks win.
  useEffect(() => {
    if (!mapping && job.suggestion) setMapping(job.suggestion.ams_mapping);
  }, [job.suggestion, mapping]);

  const used = slice.filaments;
  const totalG = used.reduce((s, f) => s + f.used_g, 0);
  // Colours as sliced, and with the trays now chosen. Filaments on one tray are
  // merged when slicing, which is what changes time and filament.
  const trayCount = (ids: (number | undefined)[]) => new Set(ids.map((t, i) => (t != null && t >= 0 ? `t${t}` : `f${i}`))).size;
  const slicedColours = slice.preferred_trays?.length ? trayCount(used.map((f) => slice.preferred_trays![f.id - 1])) : used.length;
  const chosenColours = mapping ? trayCount(used.map((f) => mapping[f.id - 1])) : slicedColours;
  const canReslice = slice.settings !== "presliced" && !job.files_removed;
  const colourCountChanged = canReslice && used.length > 0 && chosenColours !== slicedColours;
  const [cost, setCost] = useState(job.cost_estimate ?? null);
  useEffect(() => setCost(job.cost_estimate ?? null), [job.cost_estimate]);
  // Spools that don't hold enough for this print (Spoolman).
  const short = used.flatMap((f) => {
    const tray = mapping?.[f.id - 1];
    // One warning per tray, even when several filaments use it.
    if (tray == null || tray < 0 || f.id !== used.find((g) => mapping?.[g.id - 1] === tray)?.id) return [];
    const need = used.filter((g) => mapping?.[g.id - 1] === tray).reduce((s, g) => s + g.used_g, 0);
    const sp = job.spools?.[tray];
    if (sp && sp.remaining_g != null) {
      return sp.remaining_g < need ? [tr("{0} — this print needs about {1} g from it.", [spoolLine(sp).replace(/^Spoolman /, tr("Spool ")), Math.ceil(need)])] : [];
    }
    // Without Spoolman: the AMS's estimate for Bambu spools (percent of the spool).
    const t = trays.find((x) => x.id === tray);
    if (t?.remain == null) return [];
    const left = (t.remain / 100) * (t.spool_g || 1000);
    return left < need
      ? [tr("{0} has about {1} g left ({2}%) — this print needs about {3} g from it.", [trayLabel(t), Math.round(left), t.remain, Math.ceil(need)])]
      : [];
  });
  const mappedColor = useCallback(
    (filament: number, fileColor: string | null) => {
      const t = trays.find((x) => x.id === mapping?.[filament - 1]);
      return t?.color || fileColor;
    },
    [mapping, trays],
  );
  const [showLayers, setShowLayers] = useState(false);
  const st = printer ? printerState(printer.status) : null;
  const printerBusy = printer ? isBusy(printer.status) : false;
  const offline = printer ? !printer.status.connected : true;
  const [waitMinutes, setWaitMinutes] = useState(0);
  useEffect(() => {
    api
      .get<ConnectionSettings>("settings/connection")
      .then((c) => setWaitMinutes(c.wait_minutes))
      .catch(() => setWaitMinutes(0));
  }, []);

  function trayFor(id: number) {
    const t = mapping?.[id - 1];
    return trays.find((x) => x.id === t) ?? null;
  }

  async function run(fn: () => Promise<Job>, done?: string) {
    setBusy(true);
    setError(null);
    try {
      onChange(await fn());
      if (done) toast(done);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  // Offline, the suggestion only knows stale or no trays: unless a person
  // chose, let AirSlice match the trays once the printer is back.
  const body = () => ({ ams_mapping: offline && !mappingTouched ? undefined : (mapping ?? undefined), ...opts, vibration_cali: true });
  const start = () =>
    run(() => api.post<Job>(`jobs/${job.id}/print`, body()), offline ? tr("Will send when {0} is back", [printer?.name ?? tr("the printer")]) : tr("Sending to the printer"));
  const addToQueue = () => run(() => api.post<Job>(`jobs/${job.id}/queue`, body()), tr("Added to the queue"));
  const changeSettings = () => run(() => api.post<Job>(`jobs/${job.id}/reslice`));
  const sliceForTrays = (ids: number[]) => run(() => api.post<Job>(`jobs/${job.id}/slice-with-trays`, { ams_mapping: ids }));
  /** A person picked another tray. */
  function pickTray(filament: number, id: number) {
    const next = [...(mapping ?? Array(slice.filament_slots).fill(-1))];
    next[filament - 1] = id;
    setMapping(next);
    setMappingTouched(true);
    // Another number of colours changes the time and filament: slice for it.
    if (canReslice && trayCount(used.map((f) => next[f.id - 1])) !== slicedColours) {
      void sliceForTrays(next);
      return;
    }
    api.post<Job["cost_estimate"]>(`jobs/${job.id}/cost-with-trays`, { ams_mapping: next }).then((c) => setCost(c ?? null)).catch(() => undefined);
  }

  return (
    <section>
      <Preview jobId={job.id} source="sliced" printerId={slice.printer_id} colorFor={mappedColor} note={tr("As sliced, in the colours of the trays chosen below.")} />
      {!job.files_removed && (
        <button className="btn small" style={{ marginTop: 8 }} onClick={() => setShowLayers(true)}>
          {tr("Show layers")}
        </button>
      )}
      {showLayers && (
        <Sheet title={tr("Layers")} onClose={() => setShowLayers(false)}>
          <Suspense fallback={<p className="faint">{tr("Loading…")}</p>}>
            <LayerView jobId={job.id} colorFor={mappedColor} />
          </Suspense>
        </Sheet>
      )}
      <div className="facts">
        <div className="fact">
          <b>{fmtDuration(slice.prediction_secs)}</b>
          <span>{tr("print time")}</span>
        </div>
        <div className="fact">
          <b>{slice.weight_g != null ? `${slice.weight_g.toFixed(0)} g` : totalG ? `${totalG.toFixed(0)} g` : "–"}</b>
          <span>{tr("filament")}</span>
        </div>
        <div className="fact">
          <b>{chosenColours || 1}</b>
          <span>{chosenColours === 1 ? tr("colour") : tr("colours")}</span>
        </div>
        {cost && (
          <div className="fact" title={costTitle(cost)}>
            <b>≈ {money(cost)}</b>
            <span>{tr("cost")}</span>
          </div>
        )}
      </div>
      {short.map((w) => (
        <Notice key={w} tone="warn">
          {w}
        </Notice>
      ))}
      {colourCountChanged && !busy && mapping && (
        <Notice tone="warn">
          {tr("These numbers are for {0} colours, and these trays give {1}.", [slicedColours, chosenColours])}{" "}
          <button type="button" className="linkish" onClick={() => void sliceForTrays(mapping)}>
            {tr("Slice again for these trays")}
          </button>
        </Notice>
      )}

      {slice.transform && <p className="faint">{describeTransform(slice.transform)}</p>}
      {slice.tweaks && Object.keys(slice.tweaks).length > 0 && <p className="faint">{tr("Changed settings: {0}.", [describeTweaks(slice.tweaks)])}</p>}
      {slice.bed_type && (
        <p className="faint">
          {tr("Made for the {0}{1}", [plateName(plates, slice.bed_type), printer?.default_plate && printer.default_plate.toLowerCase() !== slice.bed_type.toLowerCase()
            ? tr(" — not the {0} that's usually on {1}. Swap the plate before printing.", [plateName(plates, printer.default_plate), printer.name])
            : "."])}
        </p>
      )}

      <h2>{tr("Filament")}</h2>
      {used.length === 0 ? (
        <p className="muted">The slicer didn't report filament use; the printer will use what's loaded.</p>
      ) : (
        <ul className="mapping panel" style={{ padding: "4px 16px" }}>
          {used.map((f) => {
            const t = trayFor(f.id);
            return (
              <li key={f.id} className="map-row">
                <div className="row" style={{ gap: 8 }}>
                  <Spool color={f.color} size={36} label={tr("Model filament {0}: {1}", [f.id, f.type])} />
                  <div>
                    <div style={{ fontWeight: 620 }}>{f.type || "Filament"}</div>
                    <div className="faint num">{f.used_g.toFixed(1)} g</div>
                  </div>
                </div>
                <div className="map-wire" style={{ ["--from" as string]: f.color, ["--to" as string]: t?.color || "var(--line)" }} />
                <div className="map-pick">
                  <TrayPicker
                    title={tr("Tray for filament {0} ({1})", [f.id, f.type])}
                    trays={trays}
                    value={mapping?.[f.id - 1] ?? -1}
                    onChange={(id) => pickTray(f.id, id)}
                    extra={trays.some((x) => x.id === 254) ? [] : [{ id: 254, label: tr("External spool") }]}
                    onManage={printer ? () => setShowTrays(true) : undefined}
                    spools={job.spools}
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {job.suggestion?.warnings.map((w) => (
        <Notice key={w} tone="warn">
          {w}
        </Notice>
      ))}
      {printer && (
        <p className="faint small" style={{ margin: "8px 0 0" }}>
          {tr("Wrong spool, or a tray changed?")}{" "}
          <button type="button" className="linkish" onClick={() => setShowTrays(true)}>
            {tr("Check the trays")}
          </button>
        </p>
      )}
      {printer && showTrays && <TraysSheet printer={printer} onClose={() => setShowTrays(false)} />}

      <div style={{ marginTop: 14 }}>
        <label className="check">
          <input type="checkbox" checked={opts.bed_leveling} onChange={(e) => setOpts({ ...opts, bed_leveling: e.target.checked })} />
          <span>{tr("Bed levelling")}</span>
        </label>
        <label className="check">
          <input type="checkbox" checked={opts.flow_cali} onChange={(e) => setOpts({ ...opts, flow_cali: e.target.checked })} />
          <span>{tr("Flow calibration")}</span>
        </label>
        <label className="check">
          <input type="checkbox" checked={opts.timelapse} onChange={(e) => setOpts({ ...opts, timelapse: e.target.checked })} />
          <span>{tr("Record timelapse")}</span>
        </label>
      </div>

      {printer && offline && (
        <Notice tone="warn">
          {tr("{0} is offline{1}.{2}", [printer.name, printer.status.last_error ? ` (${printer.status.last_error})` : "", waitMinutes ? tr(" AirSlice can send it as soon as the printer is back, for up to {0}.", [fmtWait(waitMinutes)]) : ""])}
        </Notice>
      )}
      {printer && printerBusy && !offline && (
        <Notice tone="warn">
          {tr("{0} is {1} right now. Add this to the queue and start it once the plate is clear.", [printer.name, st?.text.toLowerCase()])}
        </Notice>
      )}
      {!printerBusy && !offline && <Notice>{tr("Make sure the build plate is clear before you start.")}</Notice>}
      {error && <Notice tone="err">{error}</Notice>}
      {offline && waitMinutes ? (
        <>
          <button className="btn go" onClick={start} disabled={busy}>
            {tr("Send when {0} is back", [printer?.name ?? tr("the printer")])}
          </button>
          <button className="btn block" style={{ marginTop: 8 }} onClick={addToQueue} disabled={busy}>
            {tr("Add to queue instead")}
          </button>
        </>
      ) : printerBusy || offline ? (
        <button className="btn go" onClick={addToQueue} disabled={busy}>
          {tr("Add to queue for {0}", [printer?.name ?? tr("the printer")])}
        </button>
      ) : (
        <>
          <button className="btn go" onClick={start} disabled={busy}>
            {tr("Start print on {0}", [printer?.name ?? tr("the printer")])}
          </button>
          <button className="btn block" style={{ marginTop: 8 }} onClick={addToQueue} disabled={busy}>
            {tr("Add to queue instead")}
          </button>
        </>
      )}
      <button className="btn ghost block" style={{ marginTop: 8 }} onClick={changeSettings} disabled={busy}>
        {tr("Change plate or settings")}
      </button>
    </section>
  );
}

function modelPlateName(p: PlateInfo) {
  return p.name ? tr("plate {0}: {1}", [p.index, p.name]) : tr("plate {0}", [p.index]);
}

/** Slice the next plate of the file with the same settings and queue it. */
async function queueNextPlate(jobId: string, plate: PlateInfo, openJob: (id: string) => void) {
  try {
    const j = await api.post<Job>(`jobs/${jobId}/queue-next-plate`);
    toast(tr("Slicing {0}; it goes in the queue when it's done", [modelPlateName(plate)]));
    openJob(j.id);
  } catch (e) {
    toast((e as Error).message);
  }
}

function QueueNextPlate({ job, plate, openJob }: { job: Job; plate: PlateInfo; openJob: (id: string) => void }) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      className="btn block"
      style={{ marginTop: 8 }}
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await queueNextPlate(job.id, plate, openJob);
        setBusy(false);
      }}
    >
      {tr("Queue next plate ({0})", [modelPlateName(plate)])}
    </button>
  );
}

/** Start page: the last print's file has a plate nobody printed or queued yet. */
function NextPlateHint({ openJob }: { openJob: (id: string) => void }) {
  const [hint, setHint] = useState<NextPlateSuggestion | null>(null);
  const [busy, setBusy] = useState(false);
  const key = hint ? `next-plate-dismissed:${hint.job_id}:${hint.plate.index}` : "";
  const [dismissed, setDismissed] = useState<string | null>(null);
  useEffect(() => {
    api.get<NextPlateSuggestion | null>("jobs/next-plate").then(setHint, () => {});
  }, []);
  if (!hint) return null;
  let gone = dismissed === key;
  try {
    gone ||= localStorage.getItem(key) === "1";
  } catch {
    /* private mode */
  }
  if (gone) return null;
  const dismiss = () => {
    setDismissed(key);
    try {
      localStorage.setItem(key, "1");
    } catch {
      /* private mode */
    }
  };
  const thumb = hint.plate.has_thumbnail ? `api/jobs/${hint.job_id}/plates/${hint.plate.index}/thumbnail` : remoteImg(hint.model_cover);
  return (
    <section className="panel next-plate">
      <div className="row" style={{ alignItems: "flex-start" }}>
        {thumb ? <img src={thumb} alt="" /> : <div className="thumb-ph">3MF</div>}
        <div className="grow" style={{ minWidth: 0 }}>
          <p style={{ margin: 0, fontWeight: 620 }}>{tr("Print the next plate?")}</p>
          <p className="faint small" style={{ margin: "2px 0 0" }}>
            {tr("{0} of {1} hasn't been printed yet.", [modelPlateName(hint.plate).replace(/^./, (c) => c.toUpperCase()), hint.model_title])}
          </p>
        </div>
        <button className="btn ghost small" onClick={dismiss} aria-label={tr("Hide this suggestion")}>
          ✕
        </button>
      </div>
      <div className="row" style={{ marginTop: 10 }}>
        <button
          className="btn primary grow"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            await queueNextPlate(hint.job_id, hint.plate, openJob);
            setBusy(false);
          }}
        >
          {tr("Queue {0}", [modelPlateName(hint.plate)])}
        </button>
        <button className="btn" onClick={() => openJob(hint.job_id)}>
          {tr("Open last print")}
        </button>
      </div>
    </section>
  );
}

async function reprint(id: string, openJob: (id: string) => void) {
  try {
    const j = await api.post<Job>(`jobs/${id}/reprint`);
    openJob(j.id);
  } catch (e) {
    toast((e as Error).message);
  }
}

/** Object names, numbered when several share one ("t-rex.stl 1", "t-rex.stl 2"). */
function labelObjects(objects: { id: string; name: string }[]): string[] {
  const total = new Map<string, number>();
  for (const o of objects) total.set(o.name, (total.get(o.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  return objects.map((o) => {
    if ((total.get(o.name) ?? 0) < 2) return o.name;
    const n = (seen.get(o.name) ?? 0) + 1;
    seen.set(o.name, n);
    return `${o.name} ${n}`;
  });
}

export function fmtWait(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? tr("{0} h {1} min", [h, m]) : `${h} h`;
}

function Waiting({ job, printers, onChange }: { job: Job; printers: Printer[]; onChange: (j: Job) => void }) {
  const printer = printers.find((p) => p.id === job.slice?.printer_id) ?? null;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const until = job.waiting_until ? new Date(job.waiting_until * 1000) : null;

  async function stop() {
    setBusy(true);
    setError(null);
    try {
      onChange(await api.post<Job>(`jobs/${job.id}/cancel-wait`));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section>
      <div className="panel" style={{ padding: 16 }}>
        <div className="row" style={{ gap: 8 }}>
          <span className={`dot ${printer?.status.connected ? "ok" : "err"}`} />
          <b className="grow">{printer ? (printer.status.connected ? tr("{0} is back — sending", [printer.name]) : tr("{0} is offline", [printer.name])) : "Printer"}</b>
        </div>
        <p className="muted" style={{ margin: "10px 0 0" }}>
          {tr("The print goes out as soon as the printer can be reached. AirSlice tries to reach it every 15 seconds{0}.", [until ? tr(" until {0}", [until.toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" })]) : ""])}
        </p>
        {printer?.status.last_error && <p className="faint small" style={{ margin: "8px 0 0" }}>{tr("Last error: {0}", [printer.status.last_error])}</p>}
      </div>
      {error && <Notice tone="err">{error}</Notice>}
      <button className="btn block" style={{ marginTop: 12 }} onClick={stop} disabled={busy}>
        {tr("Stop waiting")}
      </button>
    </section>
  );
}

function Queued({ job, printers, onChange }: { job: Job; printers: Printer[]; onChange: (j: Job) => void }) {
  const printer = printers.find((p) => p.id === job.slice?.printer_id) ?? null;
  const [entry, setEntry] = useState<QueueEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  usePoll(async () => {
    if (!printer) return;
    try {
      const q = await api.get<QueueEntry[]>(`queue?printer_id=${printer.id}`);
      setEntry(q.find((e) => e.job_id === job.id) ?? null);
    } catch {
      /* ignore */
    }
  }, 4000, !!printer);

  const free = !!printer && printer.status.connected && !isBusy(printer.status);
  const first = entry?.position === 1;

  async function act(fn: () => Promise<Job>) {
    setBusy(true);
    setError(null);
    try {
      onChange(await fn());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section style={{ marginTop: 12 }}>
      <Notice tone={free && first ? "warn" : ""}>
        {tr("Queued on {0}{1}", [printer?.name ?? tr("the printer"), entry ? (entry.position === 1 ? tr(", next in line.") : tr(", number {0} in line.", [entry.position])) : "."])}
        {free && first && ` ${tr("{0} is free. Clear the build plate, then start.", [printer!.name])}`}
      </Notice>
      {free && first && (
        <>
          <button
            className="btn go"
            disabled={busy}
            onClick={() => act(() => api.post<Job>(`printers/${printer!.id}/start-next`))}
          >
            {tr("Plate is clear, start now")}
          </button>
        </>
      )}
      {error && <Notice tone="err">{error}</Notice>}
      <div className="row wrap" style={{ marginTop: 8 }}>
        <a className="btn grow" href="#/">
          {tr("See the queue")}
        </a>
        {entry && (
          <button
            className="btn grow"
            disabled={busy}
            onClick={async () => {
              if (!window.confirm(tr("Remove “{0}” from the queue?", [entry.title]))) return;
              setBusy(true);
              try {
                await api.del(`queue/${entry.id}`);
                onChange(await api.get<Job>(`jobs/${job.id}`));
                toast(tr("Removed from the queue"));
              } catch (e) {
                setError((e as Error).message);
              } finally {
                setBusy(false);
              }
            }}
          >
            {tr("Remove")}
          </button>
        )}
      </div>
    </section>
  );
}

function cleanTransform(t: MeshTransform): MeshTransform {
  const out: MeshTransform = {};
  if (t.scale_percent && Math.abs(t.scale_percent - 100) > 0.01) out.scale_percent = t.scale_percent;
  if (t.copies && t.copies > 1) out.copies = t.copies;
  if (t.rotate_z && t.rotate_z % 360 !== 0) out.rotate_z = t.rotate_z;
  if (t.auto_orient !== undefined) out.auto_orient = t.auto_orient;
  return out;
}

function CopiesStepper({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  return (
    <div className="tf-row">
      <span className="tf-label">{tr("Copies")}</span>
      <div className="row" style={{ gap: 6 }}>
        <button type="button" className="btn small" aria-label={tr("Fewer copies")} disabled={value <= 1} onClick={() => onChange(value - 1)}>
          −
        </button>
        <span className="tf-count num" aria-live="polite">
          {value}
        </span>
        <button type="button" className="btn small" aria-label={tr("More copies")} disabled={value >= 50} onClick={() => onChange(value + 1)}>
          +
        </button>
      </div>
    </div>
  );
}

function TransformControls({ value, onChange }: { value: MeshTransform; onChange: (t: MeshTransform) => void }) {
  const scale = value.scale_percent ?? 100;
  const copies = value.copies ?? 1;
  const rotate = value.rotate_z ?? 0;
  const orient = value.auto_orient ?? rotate === 0;
  const set = (patch: Partial<MeshTransform>) => onChange({ ...value, ...patch });
  return (
    <div className="panel transform">
      <div className="tf-row">
        <span className="tf-label">{tr("Size")}</span>
        <div className="row" style={{ gap: 6 }}>
          <button type="button" className="btn small" aria-label={tr("Smaller")} onClick={() => set({ scale_percent: Math.max(10, Math.round(scale - 10)) })}>
            −
          </button>
          <label className="tf-num">
            <input
              className="input num"
              type="number"
              inputMode="decimal"
              min={1}
              max={1000}
              value={scale}
              onChange={(e) => set({ scale_percent: Number(e.target.value) || 100 })}
              aria-label={tr("Size in percent")}
            />
            <span>%</span>
          </label>
          <button type="button" className="btn small" aria-label={tr("Larger")} onClick={() => set({ scale_percent: Math.min(1000, Math.round(scale + 10)) })}>
            +
          </button>
        </div>
      </div>
      <CopiesStepper value={copies} onChange={(c) => set({ copies: c })} />
      <div className="tf-row">
        <span className="tf-label">{tr("Rotate")}</span>
        <div className="segmented" role="group" aria-label={tr("Rotation")}>
          {[0, 90, 180, 270].map((r) => (
            <button key={r} type="button" aria-pressed={rotate === r} onClick={() => set({ rotate_z: r, auto_orient: r === 0 ? undefined : false })}>
              {r}°
            </button>
          ))}
        </div>
      </div>
      <label className="check">
        <input type="checkbox" checked={orient} onChange={(e) => set({ auto_orient: e.target.checked })} />
        <span>{tr("Lay flat automatically{0}", [rotate !== 0 && orient ? tr(" (may undo the rotation)") : ""])}</span>
      </label>
    </div>
  );
}

function Started({
  job,
  printers,
  reset,
  openJob,
  onChange,
}: {
  job: Job;
  printers: Printer[];
  reset: () => void;
  openJob: (id: string) => void;
  onChange: (j: Job) => void;
}) {
  const printer = printers.find((p) => p.id === job.print?.printer_id);
  const name = printer?.name ?? tr("your printer");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Send the same sliced file again, with the same trays, once the printer
  // isn't busy with this (or another) print.
  const canRestart = !!printer && !job.files_removed && !!job.slice && !(job.state === "started" && job.live) && !isBusy(printer.status);
  async function restart() {
    if (!window.confirm(tr("Start “{0}” again on {1}? Make sure the build plate is clear.", [job.model_title, name]))) return;
    setBusy(true);
    setError(null);
    try {
      onChange(await api.post<Job>(`jobs/${job.id}/print`, { ams_mapping: job.print?.ams_mapping, vibration_cali: true }));
      toast(printer?.status.connected ? tr("Sending to the printer") : tr("Will send when {0} is back", [name]));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  const live = job.live;
  const took = job.print?.ended_at && job.print.started_at ? fmtDuration(job.print.ended_at - job.print.started_at) : null;
  return (
    <section style={{ marginTop: 12 }}>
      {job.state === "finished" ? (
        job.print?.outcome === "unknown" ? (
          <Notice>{tr("{0} moved on to another print, so how this one went isn't known.", [name])}</Notice>
        ) : (
          <Notice tone="ok">
            {tr("Printed on {0}{1}{2}.", [name, took ? tr(" in {0}", [took]) : "", job.cost ? tr(", about {0}", [money(job.cost)]) : ""])}
            {job.print?.energy_kwh ? ` ${tr("Used {0} kWh.", [job.print.energy_kwh.toLocaleString(locale, { maximumFractionDigits: 2 })])}` : ""}
          </Notice>
        )
      ) : live ? (
        <div className="panel print-live">
          <div className="row small">
            <span className="grow" style={{ fontWeight: 620 }}>
              {live.state === "PAUSE" ? tr("Paused on {0}", [name]) : live.state === "PREPARE" ? tr("Getting ready on {0}", [name]) : tr("Printing on {0}", [name])}
            </span>
            <b className="num">{live.percent ?? 0}%</b>
          </div>
          <div className="strip-track">
            <div className="strip-fill" style={{ width: `${Math.min(100, live.percent ?? 0)}%` }} />
          </div>
          <div className="faint small num" style={{ marginTop: 8 }}>
            {fmtLeft(live.remaining_minutes)}
            {live.layer != null && live.total_layers ? tr(", layer {0} of {1}", [live.layer, live.total_layers]) : ""}
          </div>
          {printer?.camera && <CameraView printerId={printer.id} />}
          {printer && (
            <div className="controls" style={{ marginTop: 12 }}>
              <PrintControls printer={printer} />
            </div>
          )}
        </div>
      ) : (
        <Notice tone="ok">
          {tr("Sent to {0}.{1}", [name, printer && !printer.status.connected ? tr(" It's offline right now; progress shows when it's back.") : ""])}
        </Notice>
      )}
      {job.photo && !live && <img className="result-photo" src={`api/jobs/${job.id}/photo.jpg`} alt={tr("The finished print")} loading="lazy" />}
      {canRestart && (
        <button className="btn go" style={{ marginTop: 12 }} onClick={restart} disabled={busy}>
          {tr("Start again on {0}", [name])}
        </button>
      )}
      {error && <Notice tone="err">{error}</Notice>}
      <div className="row" style={{ gap: 8, marginTop: canRestart ? 8 : 12 }}>
        <button className={`btn grow ${canRestart ? "" : "primary"}`} onClick={reset}>
          {tr("New print")}
        </button>
        {job.source !== "external" && (
          <button className="btn grow" onClick={() => reprint(job.id, openJob)}>
            {tr("Change settings")}
          </button>
        )}
      </div>
    </section>
  );
}

function costTitle(c: { filament: number | null; machine: number | null; currency: string }): string {
  const parts = [];
  if (c.filament != null) parts.push(`filament ${c.filament.toFixed(2)}`);
  if (c.machine != null) parts.push(tr("print time {0}", [c.machine.toFixed(2)]));
  return parts.join(" + ") + (c.currency ? ` ${c.currency}` : "");
}

function describeTransform(t: MeshTransform): string {
  const parts: string[] = [];
  if (t.scale_percent && Math.abs(t.scale_percent - 100) > 0.01) parts.push(tr("Size {0} %", [Math.round(t.scale_percent)]));
  if (t.copies && t.copies > 1) parts.push(`${t.copies} copies`);
  if (t.rotate_z && t.rotate_z % 360 !== 0) parts.push(`rotated ${Math.round(((t.rotate_z % 360) + 360) % 360)}°`);
  return parts.join(", ");
}
