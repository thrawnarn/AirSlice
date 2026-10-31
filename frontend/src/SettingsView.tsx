import { useEffect, useState, type FormEvent } from "react";
import { api, inHomeAssistant } from "./api";
import { currentEndpoint, enablePush, pushBlocker } from "./push";
import type { ApiKey, ConnectionSettings, CostSettings, Info, NotifySettings, NotifyTarget, Printer } from "./types";
import { PrinterSettings } from "./PrintersView";
import { Notice, timeAgo, toast } from "./ui";
import { type LangChoice, langChoice, tr, setLangChoice } from "./i18n";

type Bambu = { linked: boolean; valid: boolean | null; email: string | null; region: string; updated_at: number | null };

/** The address phones and integrations should use. Inside Home Assistant
 *  the ingress URL only works in the HA app, so point at the add-on port. */
function appUrl(): string {
  if (inHomeAssistant) return `http://${window.location.hostname}:8080/`;
  return window.location.origin + window.location.pathname;
}

type SectionKey = "printers" | "language" | "sources" | "notifications" | "spoolman" | "costs" | "offline" | "storage" | "phone" | "api" | "access" | "about";

const SECTIONS: { key: SectionKey; label: string; hint: string }[] = [
  { key: "printers", label: tr("Printers"), hint: tr("Add, edit and remove printers") },
  { key: "sources", label: tr("Model sites"), hint: tr("MakerWorld, Printables and Thingiverse") },
  { key: "notifications", label: tr("Notifications"), hint: tr("Home Assistant, Telegram, ntfy or this browser") },
  { key: "spoolman", label: "Spoolman", hint: tr("Spools, what's left, prices") },
  { key: "costs", label: tr("Print costs"), hint: tr("Price per kg and per hour") },
  { key: "offline", label: tr("When the printer is offline"), hint: tr("Waiting and retries") },
  { key: "storage", label: tr("Storage"), hint: tr("Old files and disk space") },
  { key: "phone", label: tr("On your phone"), hint: tr("Home screen app and sharing") },
  { key: "api", label: tr("API keys"), hint: tr("Home Assistant and scripts") },
  { key: "access", label: tr("Access"), hint: tr("Password and sign-out") },
  { key: "language", label: tr("Language"), hint: tr("English or Swedish") },
  { key: "about", label: tr("About"), hint: tr("Versions") },
];

export function SettingsView({
  section,
  printers,
  reloadPrinters,
  passwordFromEnv,
  ingressUser,
  passwordSet,
  onSignedOut,
}: {
  /** From the address, e.g. "spoolman" or "printers/<id>"; empty shows the menu. */
  section: string;
  printers: Printer[] | null;
  reloadPrinters: () => Promise<void>;
  passwordFromEnv: boolean;
  ingressUser: string | null;
  passwordSet: boolean;
  onSignedOut: () => void;
}) {
  const [info, setInfo] = useState<Info | null>(null);
  useEffect(() => {
    api.get<Info>("info").then(setInfo).catch(() => undefined);
  }, []);
  // Phones show the menu or one section; wide screens show both side by side.
  const wide = useMediaQuery("(min-width: 860px)");
  // Earlier addresses of the model sites, from before they shared a page.
  const [head, ...rest] = section.split("/");
  const key = head === "makerworld" || head === "thingiverse" ? "sources" : head;
  const current = SECTIONS.find((x) => x.key === key) ?? (wide ? SECTIONS[0] : null);

  const body = (key: SectionKey) => {
    switch (key) {
      case "printers":
        return <PrinterSettings printers={printers} reload={reloadPrinters} open={rest[0] ?? null} />;
      case "sources":
        return (
          <>
            <BambuAccount />
            <h2 className="subsection">Printables</h2>
            <div className="panel">
              <p className="muted" style={{ margin: 0 }}>{tr("No account needed: paste a printables.com link on the Print page.")}</p>
            </div>
            <Thingiverse />
          </>
        );
      case "language":
        return <Language />;
      case "notifications":
        return <Notifications />;
      case "spoolman":
        return <SpoolmanSection />;
      case "costs":
        return <Costs />;
      case "offline":
        return <Connection />;
      case "storage":
        return <Storage />;
      case "phone":
        return <PhoneSetup />;
      case "api":
        return <ApiKeys />;
      case "access":
        return <Account passwordFromEnv={passwordFromEnv} ingressUser={ingressUser} passwordSet={passwordSet} onSignedOut={onSignedOut} />;
      case "about":
        return info ? (
          <div className="panel small">
            <p>{tr("AirSlice {0} alpha", [info.version])}</p>
            <p className={info.slicer.available ? "muted" : ""} style={{ color: info.slicer.available ? undefined : "var(--err)" }}>
              {tr("OrcaSlicer {0}{1}", [info.slicer.available ? (info.slicer.version ?? "found") : tr("not found at {0}", [info.slicer.path]), info.slicer.available && !info.slicer.profiles_loaded ? tr(", profiles missing: {0}", [info.slicer.profiles_error]) : ""])}
            </p>
            <p className="faint" style={{ margin: 0 }}>
              {tr("The printer never needs internet. This server talks to MakerWorld, Printables and Thingiverse on its behalf.")}
            </p>
            <Diagnostics />
          </div>
        ) : null;
    }
  };

  const menu = (
    <nav className="settings-menu" aria-label={tr("Settings sections")}>
      {SECTIONS.map((x) => (
        <a key={x.key} href={`#/settings/${x.key}`} aria-current={current?.key === x.key ? "page" : undefined}>
          <span className="settings-menu-label">{x.label}</span>
          <span className="settings-menu-hint">{x.hint}</span>
        </a>
      ))}
    </nav>
  );

  return (
    <main className={`page settings ${wide ? "settings-wide" : ""}`}>
      {!wide && current ? (
        <a className="settings-back" href="#/settings">
          {tr("‹ Settings")}
        </a>
      ) : null}
      <h1 className="page-title">{!wide && current ? current.label: tr("Settings")}</h1>
      <div className="settings-layout">
        {(wide || !current) && menu}
        {current && (
          <section className="settings-section" key={current.key} aria-label={current.label}>
            {wide && <h2 className="settings-section-title">{current.label}</h2>}
            {body(current.key)}
          </section>
        )}
      </div>
    </main>
  );
}

function useMediaQuery(query: string): boolean {
  const [match, setMatch] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const m = window.matchMedia(query);
    const on = () => setMatch(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, [query]);
  return match;
}

function BambuAccount() {
  const [state, setState] = useState<Bambu | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [region, setRegion] = useState("global");
  const [code, setCode] = useState("");
  const [token, setToken] = useState("");
  const [step, setStep] = useState<"form" | "code">("form");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => api.get<Bambu>("bambu").then(setState).catch(() => undefined);
  useEffect(() => {
    load();
  }, []);

  async function run(fn: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const login = (e: FormEvent) => {
    e.preventDefault();
    run(async () => {
      const r = await api.post<{ status: string; message?: string }>("bambu/login", { email, password, region });
      if (r.status === "email_code") setStep("code");
      else if (r.status === "totp") setError(r.message ?? tr("Use an access token instead."));
      else {
        toast(tr("Bambu account linked"));
        setPassword("");
        load();
      }
    });
  };

  const verify = (e: FormEvent) => {
    e.preventDefault();
    run(async () => {
      await api.post("bambu/verify", { email, code, region });
      toast(tr("Bambu account linked"));
      setStep("form");
      setPassword("");
      setCode("");
      load();
    });
  };

  return (
    <>
      <h2 className="subsection">MakerWorld</h2>
      <div className="panel">
        {state?.linked ? (
          <>
            <p>
              {tr("Linked{0}{1}", [state.email ? tr(" as {0}", [state.email]) : "", state.valid === false ? "" : "."])}
            </p>
            {state.valid === false && <Notice tone="err">{tr("The sign-in has expired. Sign in again below.")}</Notice>}
            <p className="faint">{tr("Only used to download files from MakerWorld (it only lets signed-in users download). AirSlice keeps the sign-in token, not your password. Your printer stays offline.")}</p>
            <button
              className="btn small"
              onClick={() =>
                run(async () => {
                  await api.del("bambu");
                  load();
                })
              }
            >
              {tr("Unlink account")}
            </button>
          </>
        ) : (
          <>
            <p className="muted">{tr("Sign in with your Bambu Lab account so this server can download MakerWorld files for you.")}</p>
            <details className="small" style={{ marginTop: 8 }}>
              <summary className="muted">{tr("Why is this needed?")}</summary>
              <ul className="faint explain-list">
                <li>{tr("MakerWorld only lets signed-in users download models. Bambu Studio and the Bambu Handy app sign in for the same reason.")}</li>
                <li>{tr("AirSlice downloads the files on your printer's behalf, so it needs its own sign-in. The printer itself stays offline.")}</li>
                <li>{tr("Your password goes to Bambu Lab once and isn't saved. AirSlice keeps only the sign-in token and your email address, on this server.")}</li>
                <li>{tr("The account is used for nothing else: no printers are linked to it and nothing is printed through Bambu's cloud.")}</li>
                <li>{tr("Printables needs no account. Without MakerWorld sign-in, you can still upload files or use Printables links.")}</li>
              </ul>
            </details>
          </>
        )}

        {(!state?.linked || state.valid === false) &&
          (step === "form" ? (
            <form onSubmit={login} style={{ marginTop: 12 }}>
              <label className="field">
                <span>{tr("Email")}</span>
                <input className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required />
              </label>
              <label className="field">
                <span>{tr("Password")}</span>
                <input className="input" type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
              </label>
              <label className="field">
                <span>{tr("Region")}</span>
                <select className="input" value={region} onChange={(e) => setRegion(e.target.value)}>
                  <option value="global">{tr("Global (makerworld.com)")}</option>
                  <option value="china">{tr("China (makerworld.com.cn)")}</option>
                </select>
              </label>
              <button className="btn primary block" disabled={busy}>
                {tr("Sign in")}
              </button>
            </form>
          ) : (
            <form onSubmit={verify} style={{ marginTop: 12 }}>
              <Notice>{tr("Bambu sent a code to {0}. Enter it here.", [email])}</Notice>
              <label className="field">
                <span>{tr("Code")}</span>
                <input className="input" inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} required />
              </label>
              <div className="row">
                <button className="btn primary grow" disabled={busy}>
                  {tr("Verify")}
                </button>
                <button type="button" className="btn" onClick={() => setStep("form")}>
                  {tr("Back")}
                </button>
              </div>
            </form>
          ))}

        {!state?.linked && (
          <details style={{ marginTop: 14 }}>
            <summary className="small muted">{tr("Use an access token instead")}</summary>
            <form
              style={{ marginTop: 10 }}
              onSubmit={(e) => {
                e.preventDefault();
                run(async () => {
                  await api.post("bambu/token", { token, region });
                  toast(tr("Bambu account linked"));
                  setToken("");
                  load();
                });
              }}
            >
              <p className="faint">{tr("For accounts with an authenticator app: copy the")} <code>{tr("token")}</code> {tr("cookie from makerworld.com while signed in.")}</p>
              <label className="field">
                <span>{tr("Access token")}</span>
                <input className="input" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" required />
              </label>
              <button className="btn block" disabled={busy}>
                {tr("Save token")}
              </button>
            </form>
          </details>
        )}
        {error && <Notice tone="err">{error}</Notice>}
      </div>
    </>
  );
}

function SpoolmanSection() {
  const [s, setS] = useState<{ url: string; book_usage: boolean } | null>(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [found, setFound] = useState<string | null>(null);

  useEffect(() => {
    api
      .get<{ url: string; book_usage: boolean }>("settings/spoolman")
      .then(setS)
      .catch((e) => setError((e as Error).message));
  }, []);

  async function run(kind: string, fn: () => Promise<void>) {
    setBusy(kind);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy("");
    }
  }
  const test = () =>
    run("test", async () => {
      const r = await api.post<{ spools: number }>("settings/spoolman/test", s);
      setFound(r.spools === 1 ? tr("Found 1 spool.") : tr("Found {0} spools.", [r.spools]));
    });
  const save = (e: FormEvent) => {
    e.preventDefault();
    void run("save", async () => {
      setS(await api.put("settings/spoolman", s));
      toast(s?.url ? tr("Spoolman saved") : tr("Spoolman turned off"));
    });
  };

  if (!s) return null;
  return (
    <>
      <h2>Spoolman</h2>
      <form className="panel" onSubmit={save}>
        <p className="muted" style={{ marginTop: 0 }}>
          {tr("Optional. With Spoolman, the trays show which spool is loaded and what's left, a print warns when a spool is too short, and costs use each spool's price.")}
        </p>
        <label className="field">
          <span>{tr("Spoolman address")}</span>
          <input className="input" value={s.url} onChange={(e) => setS({ ...s, url: e.target.value })} placeholder={tr("http://spoolman.local:7912 (empty = off)")} autoCapitalize="off" />
          <small>Spools are matched to trays by their RFID tag; others can be linked under Printers, the spools, Edit.</small>
        </label>
        <label className="check">
          <input type="checkbox" checked={s.book_usage} disabled={!s.url} onChange={(e) => setS({ ...s, book_usage: e.target.checked })} />
          <span>{tr("Book the filament each print used in Spoolman")}</span>
        </label>
        <p className="faint small" style={{ margin: "0 0 12px" }}>
          {tr("Leave this off if another tool (HaspelSync, the Bambu Lab integration) already books it, or it's counted twice.")}
        </p>
        {found && <Notice tone="ok">{found}</Notice>}
        {error && <Notice tone="err">{error}</Notice>}
        <div className="row" style={{ gap: 8 }}>
          <button className="btn primary" disabled={!!busy}>
            {tr("Save")}
          </button>
          {s.url && (
            <button type="button" className="btn" onClick={test} disabled={!!busy}>
              {busy === "test" ? tr("Checking…") : tr("Test")}
            </button>
          )}
        </div>
      </form>
    </>
  );
}

function Costs() {
  const [c, setC] = useState<CostSettings | null>(null);
  // Kept as typed ("1," or "1.5"), turned into numbers when saving.
  const [kg, setKg] = useState("");
  const [hour, setHour] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const show = (v: number | null) => (v == null ? "" : String(v));
  useEffect(() => {
    api
      .get<CostSettings>("settings/cost")
      .then((r) => {
        setC(r);
        setKg(show(r.price_per_kg));
        setHour(show(r.per_hour));
      })
      .catch((e) => setError((e as Error).message));
  }, []);
  const parse = (v: string): number | null | undefined => {
    const t = v.trim().replace(",", ".");
    if (t === "") return null;
    const n = Number(t);
    return Number.isFinite(n) && n >= 0 ? n : undefined;
  };
  async function save(e: FormEvent) {
    e.preventDefault();
    if (!c) return;
    const price_per_kg = parse(kg);
    const per_hour = parse(hour);
    if (price_per_kg === undefined || per_hour === undefined) {
      setError(tr("Prices are numbers, like 150 or 1,5."));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const r = await api.put<CostSettings>("settings/cost", { ...c, price_per_kg, per_hour });
      setC(r);
      setKg(show(r.price_per_kg));
      setHour(show(r.per_hour));
      toast(tr("Prices saved"));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  }
  if (!c) return null;
  return (
    <>
      <h2>{tr("Print costs")}</h2>
      <form className="panel" onSubmit={save}>
        <p className="muted" style={{ marginTop: 0 }}>
          Optional. Filament prices come from Spoolman (the spool's price, else its filament's); the price below is for spools without one.
          {tr("Prints show what they cost before you start and in the history.")}
        </p>
        <div className="row wrap" style={{ gap: 12, alignItems: "flex-start" }}>
          <label className="field" style={{ flex: "1 1 140px" }}>
            <span>{tr("Filament, per kg spool")}</span>
            <input className="input num" type="text" inputMode="decimal" value={kg} onChange={(e) => setKg(e.target.value)} placeholder="e.g. 150" />
          </label>
          <label className="field" style={{ flex: "1 1 140px" }}>
            <span>{tr("Per print hour")}</span>
            <input className="input num" type="text" inputMode="decimal" value={hour} onChange={(e) => setHour(e.target.value)} placeholder="e.g. 1,5" />
          </label>
          <label className="field" style={{ flex: "0 1 90px" }}>
            <span>{tr("Currency")}</span>
            <input className="input" value={c.currency} onChange={(e) => setC({ ...c, currency: e.target.value })} placeholder={tr("kr")} />
          </label>
        </div>
        <p className="faint small" style={{ margin: "0 0 12px" }}>
          {tr("Per print hour covers power and wear. Leave both empty (and no Spoolman prices) to hide costs.")}
        </p>
        {error && <Notice tone="err">{error}</Notice>}
        <button className="btn primary" disabled={busy}>
          {tr("Save")}
        </button>
      </form>
    </>
  );
}

type StorageInfo = { keep_files_days: number; keep_files_count: number; used_bytes: number; jobs_with_files: number; jobs_cleaned: number; free_bytes: number; total_bytes: number };

function gb(bytes: number): string {
  if (bytes < 1e9) return `${Math.round(bytes / 1e6)} MB`;
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

function Storage() {
  const [info, setInfo] = useState<StorageInfo | null>(null);
  const [days, setDays] = useState("");
  const [count, setCount] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState<string | null>(null);
  const load = () =>
    api
      .get<StorageInfo>("settings/storage")
      .then((i) => {
        setInfo(i);
        setDays(i.keep_files_days ? String(i.keep_files_days) : "");
        setCount(i.keep_files_count ? String(i.keep_files_count) : "");
      })
      .catch((e) => setError((e as Error).message));
  useEffect(() => {
    void load();
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    setBusy("save");
    setError(null);
    try {
      const n = (v: string) => Math.max(0, Math.round(Number(v) || 0));
      setInfo(await api.put<StorageInfo>("settings/storage", { keep_files_days: n(days), keep_files_count: n(count) }));
      toast(tr("Saved"));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy("");
    }
  }
  async function clean() {
    setBusy("clean");
    setError(null);
    try {
      const r = await api.post<{ jobs: number; bytes: number }>("settings/storage/clean");
      toast(r.jobs ? tr("Removed the files of {0} jobs ({1})", [r.jobs, gb(r.bytes)]) : tr("Nothing old enough to remove"));
      await load();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy("");
    }
  }

  if (!info) return null;
  return (
    <>
      <h2>{tr("Storage")}</h2>
      <form className="panel" onSubmit={save}>
        <p style={{ marginTop: 0 }}>
          {tr("AirSlice uses")} <b>{gb(info.used_bytes)}</b> for {info.jobs_with_files} jobs.
          {info.total_bytes > 0 && (
            <span className="muted">
              {tr(" {0} of {1} free on the disk.", [gb(info.free_bytes), gb(info.total_bytes)])}
            </span>
          )}
        </p>
        <div className="field">
          <span>{tr("Keep downloaded and sliced files")}</span>
          <div className="row wrap" style={{ gap: 8 }}>
            <span className="muted">{tr("for")}</span>
            <input className="input num" style={{ maxWidth: 90 }} type="number" inputMode="numeric" min={0} max={3650} value={days} onChange={(e) => setDays(e.target.value)} placeholder="–" aria-label={tr("Days")} />
            <span className="muted">{tr("days, and for the newest")}</span>
            <input className="input num" style={{ maxWidth: 90 }} type="number" inputMode="numeric" min={0} max={10000} value={count} onChange={(e) => setCount(e.target.value)} placeholder="–" aria-label={tr("Number of jobs")} />
            <span className="muted">{tr("jobs")}</span>
          </div>
          <small>
            Use one or both; whichever is reached first removes files. Empty = no limit. Favourites, queued and running jobs are always kept and
            don't count. The history stays; printing an old job again downloads it again.
          </small>
        </div>
        {error && <Notice tone="err">{error}</Notice>}
        <div className="row" style={{ gap: 8 }}>
          <button className="btn primary" disabled={!!busy}>
            {tr("Save")}
          </button>
          <button type="button" className="btn" onClick={clean} disabled={!!busy || (!Number(days) && !Number(count))}>
            {busy === "clean" ? tr("Cleaning…") : tr("Clean up now")}
          </button>
        </div>
      </form>
    </>
  );
}

function Language() {
  const choice = langChoice();
  return (
    <>
      <h2>{tr("Language")}</h2>
      <div className="panel">
        <label className="field">
          <span>{tr("Language")}</span>
          <select className="input" value={choice} onChange={(e) => setLangChoice(e.target.value as LangChoice)}>
            <option value="auto">{tr("Automatic (as on this device)")}</option>
            <option value="en">English</option>
            <option value="sv">Svenska</option>
          </select>
          <small>{tr("Saved in this browser. The page reloads to switch.")}</small>
        </label>
      </div>
    </>
  );
}

function Thingiverse() {
  const [hasToken, setHasToken] = useState<boolean | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api
      .get<{ has_token: boolean }>("settings/thingiverse")
      .then((r) => setHasToken(r.has_token))
      .catch((e) => setError((e as Error).message));
  }, []);

  async function run(fn: () => Promise<{ has_token: boolean }>, done: string) {
    setBusy(true);
    setError(null);
    try {
      setHasToken((await fn()).has_token);
      setToken("");
      toast(done);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h2 className="subsection">Thingiverse</h2>
      <div className="panel">
        {hasToken ? (
          <>
            <p>{tr("Token saved. Thingiverse links work.")}</p>
            <button className="btn small" disabled={busy} onClick={() => run(() => api.del("settings/thingiverse"), tr("Token removed"))}>
              {tr("Remove token")}
            </button>
          </>
        ) : (
          <>
            <p className="muted">{tr("Thingiverse only lets apps download with a developer token. Getting one is free and takes a minute:")}</p>
            <ol className="howto small">
              <li>
                {tr("Sign in at thingiverse.com, then open ")}
                <a href="https://www.thingiverse.com/developers" target="_blank" rel="noreferrer">
                  {tr("thingiverse.com/developers")}
                </a>
                .
              </li>
              <li>
                {tr("Choose")} <b>{tr("Create an App")}</b>{tr(". Any name works, for example AirSlice.")}
              </li>
              <li>
                {tr("Copy the app's")} <b>{tr("App Token")}</b> {tr("and paste it below.")}
              </li>
            </ol>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void run(() => api.put("settings/thingiverse", { token }), tr("Thingiverse token saved"));
              }}
            >
              <label className="field">
                <span>{tr("App token")}</span>
                <input className="input" type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" required />
                <small>{tr("Kept on this server and only sent to Thingiverse.")}</small>
              </label>
              <button className="btn primary" disabled={busy || !token.trim()}>
                {busy ? tr("Checking…") : tr("Save")}
              </button>
            </form>
          </>
        )}
        {error && <Notice tone="err">{error}</Notice>}
      </div>
    </>
  );
}

const NOTIFY_EVENTS: { key: "sliced" | "printing" | "first_layer" | "finished" | "problems" | "queue"; label: string }[] = [
  { key: "sliced", label: tr("A model is sliced and ready to print") },
  { key: "printing", label: tr("A print starts") },
  { key: "first_layer", label: tr("The first layer is done (check that it sticks)") },
  { key: "finished", label: tr("A print is done, failed or was stopped") },
  { key: "queue", label: tr("A printer is free and prints of its type are queued") },
  { key: "problems", label: tr("Something went wrong (slicing, sending, an offline printer)") },
];

const CHANNEL_NAMES: Record<NotifyTarget["channel"], string> = { ha: "Home Assistant", telegram: "Telegram", ntfy: "ntfy", webpush: tr("Browser") };

function newTarget(channel: NotifyTarget["channel"]): NotifyTarget {
  return {
    id: `new-${Date.now()}`,
    name: "",
    enabled: true,
    channel,
    service: "",
    telegram_chat: "",
    ntfy_url: "",
    photos: true,
    language: "",
    sliced: true,
    printing: true,
    first_layer: true,
    finished: true,
    problems: true,
    queue: true,
  };
}

/** Where a notification goes, in a few words. */
function targetSummary(t: NotifyTarget, mine: string | null): string {
  const to =
    t.channel === "ha"
      ? t.service.replace(/^mobile_app_/, "")
      : t.channel === "telegram"
        ? t.telegram_chat
        : t.channel === "webpush"
          ? t.push_endpoint && t.push_endpoint === mine
            ? tr("this device")
            : ""
          : t.ntfy_url.replace(/^https?:\/\//, "");
  const events = NOTIFY_EVENTS.filter((e) => t[e.key]).length;
  return [CHANNEL_NAMES[t.channel], to, tr("{0} of {1} events", [events, NOTIFY_EVENTS.length])].filter(Boolean).join(" · ");
}

/** Versions, printers, recent failures and log lines, secrets removed, to
 * paste into a message when something goes wrong. */
function Diagnostics() {
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function copy() {
    setBusy(true);
    const report = api.text("diagnostics");
    try {
      try {
        // Safari only copies within the tap: hand it the pending text.
        if (typeof ClipboardItem === "undefined" || !navigator.clipboard?.write) throw new Error("no ClipboardItem");
        await navigator.clipboard.write([new ClipboardItem({ "text/plain": report.then((t) => new Blob([t], { type: "text/plain" })) })]);
        toast(tr("Diagnostics copied"));
        setText(null);
      } catch {
        const t = await report;
        try {
          await navigator.clipboard.writeText(t);
          toast(tr("Diagnostics copied"));
          setText(null);
        } catch {
          // No clipboard (plain http, or the browser refused): copy by hand.
          setText(t);
        }
      }
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div style={{ marginTop: 14 }}>
      <button type="button" className="btn small" onClick={copy} disabled={busy}>
        {busy ? tr("Collecting…") : tr("Copy diagnostics")}
      </button>
      <p className="faint" style={{ margin: "6px 0 0" }}>
        {tr("Versions, printers, recent errors and log lines, for a bug report. Access codes, tokens and serial numbers are left out.")}
      </p>
      {text && <textarea className="input" readOnly value={text} rows={10} style={{ marginTop: 8, fontFamily: "ui-monospace, monospace", fontSize: 12 }} onFocus={(e) => e.target.select()} />}
    </div>
  );
}

function Notifications() {
  const [s, setS] = useState<NotifySettings | null>(null);
  const [services, setServices] = useState<string[] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [busy, setBusy] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  // This browser's push address, to tell which entry is this device.
  const [mine, setMine] = useState<string | null>(null);
  const blocker = pushBlocker();

  useEffect(() => {
    void currentEndpoint().then(setMine);
    api
      .get<NotifySettings>("settings/notify")
      .then((v) => {
        setS(v);
        if (v.targets.length === 0) setOpen(null);
      })
      .catch((e) => setError((e as Error).message));
  }, []);

  const usesHa = !!s?.targets.some((t) => t.channel === "ha");
  // In the add-on the list loads straight away; elsewhere once an address is set.
  useEffect(() => {
    if (s && usesHa && services === null && (s.in_addon || (s.ha_url && (s.has_token || s.token)))) void findServices(s);
  }, [s?.in_addon, usesHa]);

  async function findServices(cur = s) {
    if (!cur) return;
    setBusy("services");
    setError(null);
    try {
      setServices(await api.post<string[]>("settings/notify/services", cur));
    } catch (e) {
      setError((e as Error).message);
      setServices([]);
    } finally {
      setBusy("");
    }
  }

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!s) return;
    setBusy("save");
    setError(null);
    try {
      setS(await api.put<NotifySettings>("settings/notify", s));
      setOpen(null);
      toast(tr("Notification settings saved"));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy("");
    }
  }

  async function test(t: NotifyTarget) {
    if (!s) return;
    setBusy(`test-${t.id}`);
    setError(null);
    try {
      await api.post(`settings/notify/test?target=${encodeURIComponent(t.id)}`, s);
      toast(tr("Test sent to {0}", [t.name || CHANNEL_NAMES[t.channel]]));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy("");
    }
  }

  if (!s) return error ? <Notice tone="err">{error}</Notice> : null;
  const set = (patch: Partial<NotifySettings>) => setS({ ...s, ...patch });
  const setTarget = (id: string, patch: Partial<NotifyTarget>) => setS({ ...s, targets: s.targets.map((t) => (t.id === id ? { ...t, ...patch } : t)) });
  const add = () => {
    const t = newTarget(s.in_addon ? "ha" : "telegram");
    setS({ ...s, enabled: true, targets: [...s.targets, t] });
    setOpen(t.id);
  };
  async function notifyThisDevice() {
    if (!s) return;
    setBusy("push");
    setError(null);
    try {
      const { settings, endpoint } = await enablePush();
      // Keep unsaved edits: take only the new or renewed browser entry.
      setS({
        ...s,
        enabled: s.enabled || settings.enabled,
        targets: [
          ...s.targets.map((t) => (t.channel === "webpush" && t.push_endpoint === endpoint ? { ...t, enabled: true } : t)),
          ...settings.targets.filter((r) => !s.targets.some((t) => t.id === r.id)),
        ],
      });
      setMine(endpoint);
      toast(tr("This device gets notifications now"));
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy("");
    }
  }
  const thisDeviceAdded = !!mine && s.targets.some((t) => t.channel === "webpush" && t.push_endpoint === mine && t.enabled);
  const remove = (t: NotifyTarget) => {
    if (!window.confirm(tr("Remove the notification “{0}”?", [t.name || CHANNEL_NAMES[t.channel]]))) return;
    setS({ ...s, targets: s.targets.filter((x) => x.id !== t.id) });
  };

  return (
    <>
      <h2>{tr("Notifications")}</h2>
      <form className="panel" onSubmit={save}>
        <label className="check">
          <input type="checkbox" checked={s.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
          <span>{tr("Send notifications")}</span>
        </label>
        <p className="faint small">{tr("Add one for each person or place: a phone through Home Assistant, a Telegram chat, an ntfy topic, or this browser. Each has its own events.")}</p>

        <ul className="notify-list">
          {s.targets.map((t) => (
            <li key={t.id} className={`notify-item ${t.enabled ? "" : "off"}`}>
              <div className="row" style={{ gap: 8 }}>
                <label className="check grow" style={{ margin: 0, minWidth: 0 }}>
                  <input type="checkbox" checked={t.enabled} onChange={(e) => setTarget(t.id, { enabled: e.target.checked })} aria-label={tr("On")} />
                  <span style={{ minWidth: 0 }}>
                    <b className="ellipsis" style={{ display: "block" }}>{t.name || CHANNEL_NAMES[t.channel]}</b>
                    <small className="faint ellipsis" style={{ display: "block" }}>{targetSummary(t, mine)}</small>
                  </span>
                </label>
                <button type="button" className="btn small" onClick={() => setOpen(open === t.id ? null : t.id)} aria-expanded={open === t.id}>
                  {open === t.id ? tr("Done") : tr("Edit")}
                </button>
              </div>
              {open === t.id && (
                <TargetEditor
                  t={t}
                  inAddon={!!s.in_addon}
                  services={services}
                  busy={busy}
                  onChange={(patch) => setTarget(t.id, patch)}
                  onFindServices={() => findServices()}
                  onTest={() => test(t)}
                  onRemove={() => remove(t)}
                />
              )}
            </li>
          ))}
        </ul>
        <div style={{ display: "grid", gap: 8, marginBottom: blocker ? 6 : 14 }}>
          <button type="button" className="btn block" onClick={add}>
            {tr("Add notification")}
          </button>
          {!blocker && !thisDeviceAdded && (
            <button type="button" className="btn block" onClick={notifyThisDevice} disabled={!!busy}>
              {busy === "push" ? tr("Turning on…") : tr("Notify this device")}
            </button>
          )}
        </div>
        {blocker && <p className="faint small" style={{ margin: "0 0 14px" }}>{blocker}</p>}

        {usesHa && !s.in_addon && (
          <fieldset className="field" style={{ border: 0, padding: 0, margin: "0 0 12px" }}>
            <span>{tr("Home Assistant connection")}</span>
            <label className="field">
              <span>{tr("Home Assistant address")}</span>
              <input className="input" value={s.ha_url} onChange={(e) => set({ ha_url: e.target.value })} placeholder="http://homeassistant.local:8123" autoCapitalize="off" />
            </label>
            <label className="field">
              <span>{tr("Long-lived access token")}</span>
              <input
                className="input"
                type="password"
                value={s.token ?? ""}
                onChange={(e) => set({ token: e.target.value })}
                placeholder={s.has_token ? tr("Saved — leave empty to keep") : tr("From your Home Assistant profile, Security")}
                autoComplete="off"
              />
            </label>
          </fieldset>
        )}
        <details style={{ marginBottom: 12 }}>
          <summary className="small muted">{tr("Advanced")}</summary>
          <label className="field" style={{ marginTop: 10 }}>
            <span>{tr("Open when tapped")}</span>
            <input className="input" value={s.open_url} onChange={(e) => set({ open_url: e.target.value })} placeholder={s.in_addon ? tr("AirSlice in the Home Assistant app") : "https://…"} autoCapitalize="off" />
          </label>
        </details>
        {error && <Notice tone="err">{error}</Notice>}
        <button className="btn primary" disabled={!!busy}>
          {tr("Save")}
        </button>
      </form>
    </>
  );
}

function TargetEditor({
  t,
  inAddon,
  services,
  busy,
  onChange,
  onFindServices,
  onTest,
  onRemove,
}: {
  t: NotifyTarget;
  inAddon: boolean;
  services: string[] | null;
  busy: string;
  onChange: (patch: Partial<NotifyTarget>) => void;
  onFindServices: () => void;
  onTest: () => void;
  onRemove: () => void;
}) {
  return (
    <div className="notify-editor">
      <label className="field">
        <span>{tr("Name")}</span>
        <input className="input" value={t.name} onChange={(e) => onChange({ name: e.target.value })} placeholder={tr("e.g. My phone, Family chat")} />
      </label>
      <label className="field">
        <span>{tr("Send with")}</span>
        {t.channel === "webpush" ? (
          <input className="input" value={tr("Browser")} disabled />
        ) : (
          <select className="input" value={t.channel} onChange={(e) => onChange({ channel: e.target.value as NotifyTarget["channel"] })}>
            <option value="ha">Home Assistant</option>
            <option value="telegram">Telegram</option>
            <option value="ntfy">ntfy</option>
          </select>
        )}
        <small>
          {t.channel === "webpush"
            ? tr("This browser shows the notifications itself. No Home Assistant needed. To add another device, open AirSlice there and tap Notify this device.")
            : t.channel === "telegram"
            ? tr("A Telegram bot sends the messages, with a camera picture when the printer has one. No Home Assistant needed.")
            : t.channel === "ntfy"
              ? tr("ntfy.sh or your own ntfy server; the ntfy app shows the messages, with a camera picture. No Home Assistant needed.")
              : tr("A notify service in Home Assistant, such as the companion app on your phone.")}
        </small>
      </label>
      {t.channel === "telegram" && (
        <>
          <label className="field">
            <span>{tr("Bot token")}</span>
            <input
              className="input"
              type="password"
              value={t.telegram_token ?? ""}
              onChange={(e) => onChange({ telegram_token: e.target.value })}
              placeholder={t.has_telegram_token ? tr("Saved — leave empty to keep") : tr("From @BotFather, e.g. 123456:ABC…")}
              autoComplete="off"
            />
          </label>
          <label className="field">
            <span>{tr("Chat id")}</span>
            <input className="input" value={t.telegram_chat} onChange={(e) => onChange({ telegram_chat: e.target.value })} placeholder="123456789" autoCapitalize="off" inputMode="numeric" />
            <small>{tr("Your own id (message @userinfobot), or a group's. Send the bot a message first so it may write to you.")}</small>
          </label>
        </>
      )}
      {t.channel === "ntfy" && (
        <>
          <label className="field">
            <span>{tr("Topic address")}</span>
            <input className="input" value={t.ntfy_url} onChange={(e) => onChange({ ntfy_url: e.target.value })} placeholder="https://ntfy.sh/a-long-secret-name" autoCapitalize="off" />
            <small>{tr("Subscribe to the same topic in the ntfy app. On ntfy.sh anyone who knows the name can read it, so pick a long, hard-to-guess one.")}</small>
          </label>
          <label className="field">
            <span>{tr("Access token (optional)")}</span>
            <input
              className="input"
              type="password"
              value={t.ntfy_token ?? ""}
              onChange={(e) => onChange({ ntfy_token: e.target.value })}
              placeholder={t.has_ntfy_token ? tr("Saved — leave empty to keep") : tr("For a protected topic")}
              autoComplete="off"
            />
          </label>
        </>
      )}
      {t.channel === "ha" && (
        <label className="field">
          <span>{tr("Send to")}</span>
          {services && services.length > 0 ? (
            <select className="input" value={t.service} onChange={(e) => onChange({ service: e.target.value })}>
              <option value="">{tr("Choose…")}</option>
              {services.map((n) => (
                <option key={n} value={n}>
                  {n.startsWith("mobile_app_") ? tr("Phone: ") + n.slice(11) : n}
                </option>
              ))}
            </select>
          ) : (
            <input className="input" value={t.service} onChange={(e) => onChange({ service: e.target.value })} placeholder="mobile_app_my_phone" autoCapitalize="off" />
          )}
          <small>
            {tr("A notify service in Home Assistant.")}{" "}
            <button type="button" className="linkish" onClick={onFindServices} disabled={!!busy}>
              {busy === "services" ? tr("Looking…") : tr("Find them")}
            </button>
            {!inAddon && ` ${tr("Needs the Home Assistant connection below.")}`}
          </small>
        </label>
      )}
      <div className="field">
        <span>{tr("Tell me when")}</span>
        <div className="check-list">
          {NOTIFY_EVENTS.map((ev) => (
            <label key={ev.key} className="check">
              <input type="checkbox" checked={t[ev.key]} onChange={(e) => onChange({ [ev.key]: e.target.checked } as Partial<NotifyTarget>)} />
              <span>{ev.label}</span>
            </label>
          ))}
        </div>
      </div>
      <label className="check" style={{ marginBottom: 12 }}>
        <input type="checkbox" checked={t.photos} onChange={(e) => onChange({ photos: e.target.checked })} />
        <span>
          {t.channel === "webpush"
            ? tr("Add a picture: the camera, else the plate (not shown on iPhone)")
            : t.channel === "ha"
            ? tr("Add a picture: the camera, else the plate (Home Assistant shows one)")
            : t.channel === "ntfy"
              ? tr("Add a picture: the camera, else the plate (ntfy shows one)")
              : tr("Add pictures from the camera and of the plate")}
        </span>
      </label>
      <label className="field">
        <span>{tr("Language")}</span>
        <select className="input" value={t.language ?? ""} onChange={(e) => onChange({ language: e.target.value as NotifyTarget["language"] })}>
          <option value="">{tr("Same as Home Assistant")}</option>
          <option value="en">English</option>
          <option value="sv">Svenska</option>
        </select>
      </label>
      <div className="row wrap" style={{ gap: 8 }}>
        <button type="button" className="btn small" onClick={onTest} disabled={!!busy}>
          {busy === `test-${t.id}` ? tr("Sending…") : tr("Send a test")}
        </button>
        <button type="button" className="btn small danger" onClick={onRemove}>
          {tr("Remove")}
        </button>
      </div>
    </div>
  );
}

const CONNECTION_FIELDS: { key: keyof ConnectionSettings; label: string; unit: string; min: number; max: number; help: string }[] = [
  {
    key: "wait_minutes",
    label: tr("Wait for an offline printer"),
    unit: tr("min"),
    min: 0,
    max: 1440,
    help: tr("A print sent while the printer is off goes out when it comes back, if that's within this time. 0 sends nothing and shows an error instead."),
  },
  { key: "upload_attempts", label: tr("Upload attempts"), unit: tr("tries"), min: 1, max: 10, help: tr("Tries before an upload counts as failed. A wrong access code isn't retried.") },
  { key: "retry_delay_secs", label: tr("Pause between attempts"), unit: tr("s"), min: 1, max: 600, help: tr("Before the second try; it doubles after each failure.") },
  { key: "start_timeout_secs", label: tr("Wait for the print to start"), unit: tr("s"), min: 5, max: 600, help: tr("How long to watch for the printer to report the print as started.") },
  {
    key: "reconnect_max_secs",
    label: tr("Longest pause between reconnects"),
    unit: tr("s"),
    min: 2,
    max: 600,
    help: tr("While a printer is off, AirSlice tries less and less often, up to this. A waiting print makes it try every 15 seconds."),
  },
];

function Connection() {
  const [values, setValues] = useState<ConnectionSettings | null>(null);
  const [saved, setSaved] = useState<ConnectionSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .get<ConnectionSettings>("settings/connection")
      .then((c) => {
        setValues(c);
        setSaved(c);
      })
      .catch((e) => setError((e as Error).message));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!values) return;
    setBusy(true);
    setError(null);
    try {
      const c = await api.put<ConnectionSettings>("settings/connection", values);
      setValues(c);
      setSaved(c);
      toast(tr("Connection settings saved"));
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const changed = !!values && !!saved && CONNECTION_FIELDS.some((f) => values[f.key] !== saved[f.key]);
  return (
    <>
      <h2>{tr("When the printer is offline")}</h2>
      <form className="panel" onSubmit={save}>
        {!values && !error && <p className="muted">{tr("Loading…")}</p>}
        {values &&
          CONNECTION_FIELDS.map((f) => (
            <label key={f.key} className="field">
              <span>{f.label}</span>
              <div className="row" style={{ gap: 8 }}>
                <input
                  className="input num"
                  style={{ maxWidth: 120 }}
                  type="number"
                  inputMode="numeric"
                  min={f.min}
                  max={f.max}
                  value={values[f.key]}
                  onChange={(e) => setValues({ ...values, [f.key]: Math.round(Number(e.target.value)) })}
                />
                <span className="muted">{f.unit}</span>
              </div>
              <small>{f.help}</small>
            </label>
          ))}
        {error && <Notice tone="err">{error}</Notice>}
        {values && (
          <div className="row" style={{ gap: 8 }}>
            <button className="btn primary" disabled={busy || !changed}>
              {tr("Save")}
            </button>
            {changed && saved && (
              <button type="button" className="btn ghost" onClick={() => setValues(saved)} disabled={busy}>
                {tr("Undo")}
              </button>
            )}
          </div>
        )}
      </form>
    </>
  );
}

function PhoneSetup() {
  const url = appUrl();
  const isIos = /iPhone|iPad|iPod/.test(navigator.userAgent);
  return (
    <>
      <h2>{tr("On your phone")}</h2>
      <div className="panel">
        {inHomeAssistant && (
          <Notice>
            {tr("For the home-screen app and shared links, open AirSlice directly at")} <code>{url}</code> {tr("(or through your reverse proxy) instead of through Home Assistant. Enable the port under the add-on's Network settings and set a password below.")}
          </Notice>
        )}
        <h3>{tr("Add to Home Screen")}</h3>
        <p className="muted small">
          {isIos
            ? tr("In Safari, tap Share, then Add to Home Screen. AirSlice opens full screen like an app.")
            : tr("In your browser menu, choose Install app or Add to Home screen. On Android, AirSlice then appears in the share sheet.")}
        </p>
        <h3 style={{ marginTop: 16 }}>{tr("Share links from MakerWorld, Printables and Thingiverse (iPhone)")}</h3>
        <p className="muted small">{tr("iOS doesn't let web apps appear in the share sheet, so a two-step Shortcut does it:")}</p>
        <ol className="howto small">
          <li>Open Shortcuts and create a new shortcut named <b>{tr("Print with AirSlice")}</b>.</li>
          <li>{tr("Tap the info button and turn on")} <b>{tr("Show in Share Sheet")}</b>; set it to receive <b>{tr("URLs")}</b> {tr("and")} <b>{tr("Text")}</b>.</li>
          <li>
            {tr("Add")} <b>{tr("URL")}</b> {tr("with:")} <code>{url}?url=</code> {tr("followed by the")} <b>{tr("Shortcut Input")}</b> {tr("variable.")}
          </li>
          <li>{tr("Add")} <b>{tr("Open URLs")}</b>.</li>
        </ol>
        <p className="faint small" style={{ marginTop: 8 }}>
          {tr("Sharing a model now opens AirSlice with the link filled in. Any link of the form")} <code>{url}?url=…</code> {tr("works the same way.")}
        </p>
      </div>
    </>
  );
}

function ApiKeys() {
  const [keys, setKeys] = useState<ApiKey[] | null>(null);
  const [name, setName] = useState("");
  const [fresh, setFresh] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = () => api.get<ApiKey[]>("keys").then(setKeys).catch(() => undefined);
  useEffect(() => {
    load();
  }, []);

  async function create(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      const r = await api.post<{ key: string }>("keys", { name });
      setFresh(r.key);
      setName("");
      load();
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function remove(k: ApiKey) {
    if (!window.confirm(tr("Delete the key “{0}”? Anything using it stops working.", [k.name]))) return;
    await api.del(`keys/${k.id}`);
    load();
  }

  return (
    <>
      <h2>{tr("API keys")}</h2>
      <div className="panel">
        <p className="muted small">{tr("For Home Assistant and other integrations. Send the key as")} <code>{tr("X-API-Key")}</code> {tr("or")} <code>{tr("Authorization: Bearer")}</code>.</p>
        {fresh && (
          <>
            <Notice tone="ok">{tr("Copy this key now. It won't be shown again.")}</Notice>
            <div className="key-reveal">{fresh}</div>
            <div className="row" style={{ marginTop: 8 }}>
              <button
                className="btn small"
                onClick={() => navigator.clipboard?.writeText(fresh).then(() => toast(tr("Key copied")))}
              >
                {tr("Copy key")}
              </button>
              <button className="btn small ghost" onClick={() => setFresh(null)}>
                {tr("Done")}
              </button>
            </div>
          </>
        )}
        {keys && keys.length > 0 && (
          <ul className="list" style={{ marginTop: 8 }}>
            {keys.map((k) => (
              <li key={k.id}>
                <div className="grow" style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 620 }}>{k.name}</div>
                  <div className="faint">
                    <code>{k.prefix}…</code> {tr("used {0}", [timeAgo(k.last_used_at)])}
                  </div>
                </div>
                <button className="btn small danger" onClick={() => remove(k)}>
                  {tr("Delete")}
                </button>
              </li>
            ))}
          </ul>
        )}
        <form onSubmit={create} className="row" style={{ marginTop: 12 }}>
          <input className="input grow" placeholder="Home Assistant" value={name} onChange={(e) => setName(e.target.value)} aria-label={tr("Key name")} />
          <button className="btn primary" disabled={!name.trim()}>
            {tr("Create key")}
          </button>
        </form>
        {error && <Notice tone="err">{error}</Notice>}
        <details style={{ marginTop: 12 }}>
          <summary className="small muted">{tr("Example: print a link with one request")}</summary>
          <pre className="key-reveal" style={{ whiteSpace: "pre-wrap", marginTop: 8 }}>
            {`curl -X POST ${appUrl()}api/print \\
  -H "X-API-Key: as_…" -H "Content-Type: application/json" \\
  -d '{"url": "https://makerworld.com/en/models/…"}'`}
          </pre>
          <p className="faint small">{tr("Then poll")} <code>GET api/jobs/&lt;id&gt;</code>{tr(". Printer status:")} <code>{tr("GET api/printers")}</code>.</p>
        </details>
      </div>
    </>
  );
}

function Account({
  passwordFromEnv,
  ingressUser,
  passwordSet,
  onSignedOut,
}: {
  passwordFromEnv: boolean;
  ingressUser: string | null;
  passwordSet: boolean;
  onSignedOut: () => void;
}) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isSet, setIsSet] = useState(passwordSet);
  const firstFromHa = !!ingressUser && !isSet;

  async function change(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      await api.post("auth/password", { current: firstFromHa ? "" : current, new: next });
      toast(firstFromHa ? tr("Password set") : tr("Password changed"));
      setIsSet(true);
      setCurrent("");
      setNext("");
    } catch (err) {
      setError((err as Error).message);
    }
  }

  return (
    <>
      <h2>{tr("Access")}</h2>
      <div className="panel">
        {ingressUser && <p className="small">{tr("Signed in through Home Assistant as {0}.", [ingressUser])}</p>}
        {passwordFromEnv ? (
          <p className="muted small">{tr("The password is set with ADMIN_PASSWORD in the container settings.")}</p>
        ) : (
          <details open={firstFromHa}>
            <summary className="small muted">
              {firstFromHa ? tr("Set a password for direct access") : tr("Change password")}
            </summary>
            <form onSubmit={change} style={{ marginTop: 10 }}>
              {firstFromHa && (
                <p className="faint small">{tr("Needed when you open AirSlice on its own port, for example from your phone's home screen.")}</p>
              )}
              {!firstFromHa && (
                <label className="field">
                  <span>{tr("Current password")}</span>
                  <input className="input" type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} required />
                </label>
              )}
              <label className="field">
                <span>{tr("New password")}</span>
                <input className="input" type="password" autoComplete="new-password" minLength={8} value={next} onChange={(e) => setNext(e.target.value)} required />
              </label>
              {error && <Notice tone="err">{error}</Notice>}
              <button className="btn primary block">{firstFromHa ? tr("Set password") : tr("Change password")}</button>
            </form>
          </details>
        )}
        <div className="row wrap" style={{ marginTop: 14 }}>
          {!ingressUser && (
            <button
              className="btn small"
              onClick={async () => {
                await api.post("auth/logout");
                onSignedOut();
              }}
            >
              {tr("Sign out")}
            </button>
          )}
          <button
            className="btn small ghost"
            onClick={async () => {
              if (!window.confirm(tr("Sign out every browser that uses the password? API keys keep working."))) return;
              await api.del("auth/sessions");
              onSignedOut();
            }}
          >
            {tr("Sign out everywhere")}
          </button>
        </div>
      </div>
    </>
  );
}
