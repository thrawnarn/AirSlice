import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { Job, ModelFile, Printer, ResolvedModel } from "./types";
import { Notice, Spool, fmtDuration, fmtSize, remoteImg } from "./ui";
import { locale, tr } from "./i18n";

export const SOURCE_NAMES: Record<string, string> = { makerworld: "MakerWorld", printables: "Printables", thingiverse: "Thingiverse" };

export function shortModel(m: string | null | undefined) {
  return (m ?? "").replace(/^Bambu Lab /i, "").toLowerCase();
}

function count(n: number) {
  return n >= 10_000 ? `${(n / 1000).toLocaleString(locale, { maximumFractionDigits: 0 })} k` : n.toLocaleString(locale);
}

/** A model: pictures, facts, description, then its files and Print. Used on
 * the print page after a link and on a model's own page (Explore). */
/** How often each file of this model was printed (from the history). */
function usePrinted(model: ResolvedModel): Map<string, number> | null {
  const [printed, setPrinted] = useState<Map<string, number> | null>(null);
  useEffect(() => {
    api.get<Job[]>("jobs?limit=1000").then(
      (jobs) => {
        const m = new Map<string, number>();
        for (const j of jobs) {
          if (j.source === model.source && j.model_id === model.model_id && (j.state === "finished" || j.state === "started")) m.set(j.file.id, (m.get(j.file.id) ?? 0) + 1);
        }
        setPrinted(m);
      },
      () => setPrinted(new Map()),
    );
  }, [model.source, model.model_id]);
  return printed;
}

export function ModelPanel({ model, defaultPrinter, openJob }: { model: ResolvedModel; defaultPrinter: Printer | null; openJob: (id: string) => void }) {
  const mine = shortModel(defaultPrinter?.printer_model);
  const [picked, setPicked] = useState<string | null>(() => {
    const forMine = model.files.find((f) => shortModel(f.printer) === mine);
    return model.preselected_file_id ?? forMine?.id ?? model.files[0]?.id ?? null;
  });
  const printed = usePrinted(model);
  const touched = useRef(false);
  // Coming back to a model (say from the history) is usually for another
  // file: move the first choice off one that was printed already.
  useEffect(() => {
    if (!printed || touched.current || !picked || !printed.has(picked)) return;
    const rest = model.files.filter((f) => !printed.has(f.id));
    const next = rest.find((f) => shortModel(f.printer) === mine) ?? rest[0];
    if (next) setPicked(next.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [printed]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create() {
    if (!picked) return;
    setBusy(true);
    setError(null);
    try {
      const job = await api.post<Job>("jobs", { url: model.url, file_id: picked });
      openJob(job.id);
    } catch (e) {
      setError((e as Error).message);
      setBusy(false);
    }
  }

  const d = model.details;
  const images = d?.images.length ? d.images : model.cover ? [model.cover] : [];
  return (
    <>
      <Gallery images={images} />
      <div className="model-name">
        <div>
          <p className="model-title">{model.title}</p>
          <p className="faint" style={{ margin: 0 }}>
            {model.author ? tr("by {0} on ", [model.author]) : tr("On ")}
            <a href={model.url} target="_blank" rel="noreferrer">
              {SOURCE_NAMES[model.source] ?? model.source}
            </a>
          </p>
        </div>
      </div>
      {d && <Facts details={d} />}
      {model.notes.map((n) => (
        <Notice key={n} tone={n.includes("Settings") ? "warn" : ""}>
          {n.includes("Settings") ? (
            <>
              {n} <a href="#/settings/sources">{tr("Open settings")}</a>
            </>
          ) : (
            n
          )}
        </Notice>
      ))}
      {model.files.length > 0 && (
        <>
          <h2>{model.source === "makerworld" ? tr("Choose a print profile") : tr("Choose a file")}</h2>
          <ul className="files">
            {model.files.map((f) => (
              <li key={f.id}>
                <FileRow
                  file={f}
                  selected={picked === f.id}
                  onSelect={() => {
                    touched.current = true;
                    setPicked(f.id);
                  }}
                  printer={defaultPrinter}
                  printed={printed?.get(f.id) ?? 0}
                />
              </li>
            ))}
          </ul>
          {error && <Notice tone="err">{error}</Notice>}
          <button className="btn go" onClick={create} disabled={!picked || busy} style={{ marginTop: 8 }}>
            {busy ? tr("Starting download…") : tr("Print this")}
          </button>
        </>
      )}
      {d && (d.summary || d.description.length > 0) && <Description summary={d.summary} blocks={d.description} />}
      {d && (d.tags.length > 0 || d.categories.length > 0) && (
        <div className="model-tags">
          {[...d.categories, ...d.tags].slice(0, 24).map((t, i) => (
            <span key={`${t}-${i}`} className="tag">
              {t}
            </span>
          ))}
        </div>
      )}
    </>
  );
}

/** Pictures in a row that scrolls sideways; the first one large. */
function Gallery({ images }: { images: string[] }) {
  if (images.length === 0) return null;
  return (
    <div className="gallery" role="list" aria-label={tr("Pictures")}>
      {images.map((u, i) => (
        <img key={u} role="listitem" src={remoteImg(u)} alt="" loading={i < 2 ? "eager" : "lazy"} />
      ))}
    </div>
  );
}

function Facts({ details: d }: { details: NonNullable<ResolvedModel["details"]> }) {
  const facts: [string, string][] = [];
  if (d.likes != null) facts.push([tr("Likes"), count(d.likes)]);
  if (d.downloads != null) facts.push([tr("Downloads"), count(d.downloads)]);
  if (d.prints != null) facts.push([tr("Prints"), count(d.prints)]);
  if (d.collections != null) facts.push([tr("Collected"), count(d.collections)]);
  const extra = [
    d.published_at ? tr("Published {0}", [new Date(d.published_at * 1000).toLocaleDateString(locale, { year: "numeric", month: "short", day: "numeric" })]) : null,
    d.license,
  ].filter(Boolean);
  return (
    <>
      {facts.length > 0 && (
        <div className="temps model-facts">
          {facts.map(([k, v]) => (
            <div key={k}>
              <span>{k}</span>
              <b className="num">{v}</b>
            </div>
          ))}
        </div>
      )}
      {extra.length > 0 && <p className="faint small" style={{ margin: "6px 0 0" }}>{extra.join(" · ")}</p>}
    </>
  );
}

/** The description as text blocks (never the site's HTML); long ones fold. */
function Description({ summary, blocks }: { summary: string | null; blocks: { kind: string; text: string }[] }) {
  const [open, setOpen] = useState(false);
  const FOLD = 6;
  const shown = open ? blocks : blocks.slice(0, FOLD);
  return (
    <section className="panel model-description">
      <h2 className="small">{tr("About this model")}</h2>
      {summary && <p className="model-summary">{summary}</p>}
      {groups(shown).map((g, i) =>
        g.kind === "li" ? (
          <ul key={i}>
            {g.items.map((t, j) => (
              <li key={j}>{t}</li>
            ))}
          </ul>
        ) : g.kind === "h" ? (
          <h3 key={i}>{g.items[0]}</h3>
        ) : (
          <p key={i}>{g.items[0]}</p>
        ),
      )}
      {blocks.length > FOLD && (
        <button type="button" className="linkish small" onClick={() => setOpen(!open)}>
          {open ? tr("Show less") : tr("Show all")}
        </button>
      )}
    </section>
  );
}

/** Consecutive list items become one list. */
function groups(blocks: { kind: string; text: string }[]): { kind: string; items: string[] }[] {
  const out: { kind: string; items: string[] }[] = [];
  for (const b of blocks) {
    const last = out[out.length - 1];
    if (b.kind === "li" && last?.kind === "li") last.items.push(b.text);
    else out.push({ kind: b.kind, items: [b.text] });
  }
  return out;
}

export function FileRow({ file, selected, onSelect, printer, printed = 0 }: { file: ModelFile; selected: boolean; onSelect: () => void; printer: Printer | null; printed?: number }) {
  const mine = shortModel(printer?.printer_model);
  const madeForMine = !!mine && shortModel(file.printer) === mine;
  const compatible = !!mine && file.compatible.some((c) => shortModel(c) === mine);
  const meta: string[] = [];
  if (file.printer) meta.push(tr("for {0}", [file.printer]));
  if (file.plates) meta.push(file.plates === 1 ? tr("1 plate") : tr("{0} plates", [file.plates]));
  if (file.prediction_secs) meta.push(fmtDuration(file.prediction_secs));
  if (file.weight_g) meta.push(`${Math.round(file.weight_g)} g`);
  if (file.size_bytes) meta.push(fmtSize(file.size_bytes));
  if (file.folder) meta.push(file.folder);
  return (
    <button className="file" aria-pressed={selected} onClick={onSelect} type="button">
      {file.thumbnail ? <img src={remoteImg(file.thumbnail)} alt="" loading="lazy" /> : <div className="thumb-ph">{file.kind.toUpperCase()}</div>}
      <div style={{ minWidth: 0 }}>
        <div className="file-name">{file.name}</div>
        <div className="file-meta">{meta.join(", ")}</div>
        <div className="row wrap" style={{ gap: 6, marginTop: 6 }}>
          {madeForMine && <span className="tag match">{tr("Made for your printer")}</span>}
          {!madeForMine && compatible && <span className="tag match">{tr("Compatible")}</span>}
          {printed > 0 && <span className="tag">{printed === 1 ? tr("Printed") : tr("Printed {0} times", [printed])}</span>}
          {file.need_ams && <span className="tag">{tr("Needs AMS")}</span>}
          {file.filaments.length > 0 && (
            <span className="row" style={{ gap: 2 }} aria-label={tr("{0} filaments", [file.filaments.length])}>
              {file.filaments.slice(0, 8).map((f, i) => (
                <Spool key={i} color={f.color} size={18} label={f.type} />
              ))}
            </span>
          )}
        </div>
      </div>
    </button>
  );
}

/** A model's own page (from Explore): look it up, then the panel. */
export function ModelPage({ url, printers, openJob }: { url: string; printers: Printer[] | null; openJob: (id: string) => void }) {
  const [model, setModel] = useState<ResolvedModel | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.post<ResolvedModel>("resolve", { url }).then(setModel, (e) => setError((e as Error).message));
  }, [url]);
  const def = printers?.find((p) => p.is_default) ?? printers?.[0] ?? null;
  return (
    <main className="page">
      <a href="#/explore" className="settings-back" onClick={(e) => { if (window.history.length > 1) { e.preventDefault(); window.history.back(); } }}>
        {tr("‹ Back")}
      </a>
      {error && <Notice tone="err">{error}</Notice>}
      {!model && !error && <p className="faint">{tr("Loading…")}</p>}
      {model && <ModelPanel model={model} defaultPrinter={def} openJob={openJob} />}
    </main>
  );
}
