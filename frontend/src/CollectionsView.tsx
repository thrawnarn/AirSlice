import { useEffect, useState } from "react";
import { api } from "./api";
import { Notice, remoteImg } from "./ui";
import { locale, tr } from "./i18n";

interface Collection {
  id: string;
  title: string;
  count: number | null;
  covers: string[];
  kind: "mine" | "liked" | "downloads";
  owner: string | null;
}

interface CollectionModel {
  id: string;
  title: string;
  cover: string | null;
  author: string | null;
  url: string;
  prints: number | null;
}

const PAGE = 24;

/** The list, kept while the app is open: opening a collection needs its name,
 * and each fetch costs three MakerWorld requests. */
let listCache: Collection[] | null = null;
async function loadList(): Promise<Collection[]> {
  if (!listCache) listCache = await api.get<Collection[]>("makerworld/collections");
  return listCache;
}

function NotLinked({ message }: { message: string }) {
  return (
    <Notice tone={message.includes("Settings") ? "warn" : "err"}>
      {message}{" "}
      {message.includes("Settings") && <a href="#/settings/sources">{tr("Open settings")}</a>}
    </Notice>
  );
}

/** The account's MakerWorld collections. */
export function CollectionsView({ id, openModel }: { id: string | null; openModel: (url: string) => void }) {
  return id ? <CollectionModels id={id} openModel={openModel} /> : <CollectionList />;
}

function CollectionList() {
  const [list, setList] = useState<Collection[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    loadList().then(setList, (e) => setError((e as Error).message));
  }, []);
  const groups: [string, Collection[]][] = [
    [tr("Your collections"), list?.filter((c) => c.kind === "mine") ?? []],
    [tr("Collections you like"), list?.filter((c) => c.kind === "liked") ?? []],
    [tr("Downloaded"), list?.filter((c) => c.kind === "downloads") ?? []],
  ];
  return (
    <main className="page">
      <h1 className="page-title">{tr("Explore")}</h1>
      <p className="faint" style={{ marginTop: -6 }}>{tr("Your MakerWorld collections. Tap a model to read about it and print it.")}</p>
      {error && <NotLinked message={error} />}
      {!list && !error && <p className="faint">{tr("Loading…")}</p>}
      {list && list.length === 0 && <p className="faint">{tr("No collections on MakerWorld yet.")}</p>}
      {groups
        .filter(([, items]) => items.length > 0)
        .map(([title, items]) => (
          <section key={title}>
            <h2 className="small">{title}</h2>
            <ul className="collections">
              {items.map((c) => (
                <li key={c.id}>
                  <a href={`#/explore/c/${c.id}`} className="collection-card">
                    <div className={`collection-covers n${Math.min(4, c.covers.length)}`}>
                      {c.covers.length ? c.covers.slice(0, 4).map((u) => <img key={u} src={remoteImg(u)} alt="" loading="lazy" />) : <div className="thumb-ph" />}
                    </div>
                    <div className="collection-meta">
                      <span className="ellipsis collection-title">{c.kind === "downloads" ? tr("Downloads") : c.title}</span>
                      <span className="faint small">
                        {[c.count != null ? (c.count === 1 ? tr("1 model") : tr("{0} models", [c.count.toLocaleString(locale)])) : null, c.owner ? tr("by {0}", [c.owner]) : null]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </div>
                  </a>
                </li>
              ))}
            </ul>
          </section>
        ))}
    </main>
  );
}

function CollectionModels({ id, openModel }: { id: string; openModel: (url: string) => void }) {
  const [models, setModels] = useState<CollectionModel[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [title, setTitle] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Where the next page starts on MakerWorld's side, and whether there is one:
  // a short page ends it even when `total` counts hidden or deleted models.
  const [next, setNext] = useState(0);
  const [more, setMore] = useState(false);

  async function load(offset: number) {
    setBusy(true);
    try {
      const page = await api.get<{ total: number; models: CollectionModel[] }>(`makerworld/collections/${encodeURIComponent(id)}?offset=${offset}&limit=${PAGE}`);
      setModels((m) => (offset === 0 ? page.models : [...m, ...page.models.filter((x) => !m.some((y) => y.id === x.id))]));
      setTotal(page.total);
      setNext(offset + PAGE);
      setMore(page.models.length >= PAGE && offset + PAGE < page.total);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    void load(0);
    // The name, from the list (cheap: one request, no models).
    loadList()
      .then((l) => {
        const c = l.find((x) => x.id === id);
        setTitle(c ? (c.kind === "downloads" ? tr("Downloads") : c.title) : "");
      })
      .catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  return (
    <main className="page">
      <a href="#/explore" className="settings-back">
        {tr("‹ Collections")}
      </a>
      <h1 className="page-title">{title || tr("Collection")}</h1>
      {total != null && <p className="faint small" style={{ marginTop: -6 }}>{total === 1 ? tr("1 model") : tr("{0} models", [total.toLocaleString(locale)])}</p>}
      {error && <NotLinked message={error} />}
      <ul className="model-grid">
        {models.map((m) => (
          <li key={m.id}>
            <button type="button" className="model-card" onClick={() => openModel(m.url)}>
              {m.cover ? <img src={remoteImg(m.cover)} alt="" loading="lazy" /> : <div className="thumb-ph" />}
              <span className="model-card-title">{m.title}</span>
              <span className="faint small ellipsis">
                {[m.author, m.prints ? tr("{0} prints", [m.prints.toLocaleString(locale)]) : null].filter(Boolean).join(" · ")}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {more && (
        <button className="btn block" style={{ marginTop: 12 }} disabled={busy} onClick={() => load(next)}>
          {busy ? tr("Loading…") : tr("Show more")}
        </button>
      )}
      {total == null && busy && <p className="faint">{tr("Loading…")}</p>}
    </main>
  );
}

/** The same collections inside a dialog: pick a model instead of opening its page. */
export function CollectionPicker({ disabled, onPick }: { disabled: boolean; onPick: (url: string) => void }) {
  const [list, setList] = useState<Collection[] | null>(null);
  const [open, setOpen] = useState<Collection | null>(null);
  const [models, setModels] = useState<CollectionModel[]>([]);
  const [more, setMore] = useState(false);
  const [next, setNext] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    loadList().then(setList, (e) => setError((e as Error).message));
  }, []);

  async function load(c: Collection, offset: number) {
    setBusy(true);
    setError(null);
    try {
      const page = await api.get<{ total: number; models: CollectionModel[] }>(`makerworld/collections/${encodeURIComponent(c.id)}?offset=${offset}&limit=${PAGE}`);
      setModels((m) => (offset === 0 ? page.models : [...m, ...page.models.filter((x) => !m.some((y) => y.id === x.id))]));
      setNext(offset + PAGE);
      setMore(page.models.length >= PAGE && offset + PAGE < page.total);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  const name = (c: Collection) => (c.kind === "downloads" ? tr("Downloads") : c.title);

  if (open) {
    return (
      <>
        <button type="button" className="linkish settings-back" onClick={() => { setOpen(null); setModels([]); setError(null); }}>
          {tr("‹ Collections")}
        </button>
        <h3 style={{ margin: "4px 0 10px" }}>{name(open)}</h3>
        {error && <NotLinked message={error} />}
        <ul className="model-grid">
          {models.map((m) => (
            <li key={m.id}>
              <button type="button" className="model-card" disabled={disabled} onClick={() => onPick(m.url)}>
                {m.cover ? <img src={remoteImg(m.cover)} alt="" loading="lazy" /> : <div className="thumb-ph" />}
                <span className="model-card-title">{m.title}</span>
                {m.author && <span className="faint small ellipsis">{m.author}</span>}
              </button>
            </li>
          ))}
        </ul>
        {busy && <p className="faint">{tr("Loading…")}</p>}
        {!busy && !error && models.length === 0 && <p className="faint">{tr("No models here.")}</p>}
        {more && !busy && (
          <button className="btn block" style={{ marginTop: 12 }} onClick={() => load(open, next)}>
            {tr("Show more")}
          </button>
        )}
      </>
    );
  }
  return (
    <>
      {error && <NotLinked message={error} />}
      {!list && !error && <p className="faint">{tr("Loading…")}</p>}
      {list && list.length === 0 && <p className="faint">{tr("No collections on MakerWorld yet.")}</p>}
      <ul className="collections">
        {list?.map((c) => (
          <li key={`${c.kind}-${c.id}`}>
            <button type="button" className="collection-card" onClick={() => { setOpen(c); void load(c, 0); }}>
              <div className={`collection-covers n${Math.min(4, c.covers.length)}`}>
                {c.covers.length ? c.covers.slice(0, 4).map((u) => <img key={u} src={remoteImg(u)} alt="" loading="lazy" />) : <div className="thumb-ph" />}
              </div>
              <div className="collection-meta">
                <span className="ellipsis collection-title">{name(c)}</span>
                {c.count != null && <span className="faint small">{c.count === 1 ? tr("1 model") : tr("{0} models", [c.count.toLocaleString(locale)])}</span>}
              </div>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}
