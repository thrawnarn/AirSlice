import { useCallback, useEffect, useState } from "react";
import { api, findModelLink, inHomeAssistant, setUnauthorizedHandler } from "./api";
import type { Info, Printer } from "./types";
import { Icon, ToastHost, usePoll } from "./ui";
import { Login } from "./Login";
import { PrintView } from "./PrintView";
import { PrintersView } from "./PrintersView";
import { HistoryView } from "./HistoryView";
import { StatsView } from "./StatsView";
import { SensorsView } from "./SensorsView";
import { CollectionsView } from "./CollectionsView";
import { ModelPage } from "./ModelView";
import { SettingsView } from "./SettingsView";
import { tr } from "./i18n";

type Auth = {
  setup_required: boolean;
  /** Add-on: the first password can only be set through Home Assistant. */
  setup_blocked?: boolean;
  authenticated: boolean;
  password_from_env: boolean;
  ingress?: boolean;
  user?: string | null;
  admin_password_set?: boolean;
};


function useHashRoute(): [string, (r: string) => void] {
  const read = () => window.location.hash.replace(/^#/, "") || "/";
  const [route, setRoute] = useState(read);
  useEffect(() => {
    const on = () => setRoute(read());
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  return [route, (r: string) => (window.location.hash = r)];
}

/** A link shared into the app (?url=, ?text= from the share sheet or an iOS Shortcut). */
function takeSharedLink(): string | null {
  const q = new URLSearchParams(window.location.search);
  const found = findModelLink(q.get("url")) ?? findModelLink(q.get("text")) ?? findModelLink(q.get("title"));
  if (q.has("url") || q.has("text") || q.has("title")) {
    // Clean the address bar so a reload doesn't import twice.
    window.history.replaceState(null, "", window.location.pathname + window.location.hash);
  }
  return found;
}

export function App() {
  const [auth, setAuth] = useState<Auth | null>(null);
  const [route, go] = useHashRoute();
  const [printers, setPrinters] = useState<Printer[] | null>(null);
  const [shared, setShared] = useState<string | null>(() => takeSharedLink());

  const checkAuth = useCallback(async () => {
    try {
      setAuth(await api.get<Auth>("auth/status"));
    } catch {
      setAuth({ setup_required: false, authenticated: false, password_from_env: false });
    }
  }, []);

  useEffect(() => {
    checkAuth();
    setUnauthorizedHandler(() => setAuth((a) => (a ? { ...a, authenticated: false } : a)));
    if ("serviceWorker" in navigator && window.isSecureContext && !inHomeAssistant) {
      navigator.serviceWorker.register("./sw.js").catch(() => undefined);
    }
  }, [checkAuth]);

  const loadPrinters = useCallback(async () => {
    try {
      setPrinters(await api.get<Printer[]>("printers"));
    } catch {
      /* shown where it matters */
    }
  }, []);
  usePoll(loadPrinters, 4000, !!auth?.authenticated);

  // What the server runs (not what the browser cached), shown at the bottom.
  const [info, setInfo] = useState<Info | null>(null);
  useEffect(() => {
    if (auth?.authenticated) api.get<Info>("info").then(setInfo).catch(() => undefined);
  }, [auth?.authenticated]);

  if (!auth) return null;
  if (!auth.authenticated) {
    return <Login setup={auth.setup_required} setupBlocked={!!auth.setup_blocked} onDone={checkAuth} />;
  }

  const section = route.startsWith("/printers")
    ? "printers"
    : route.startsWith("/history")
      ? "history"
      : route.startsWith("/settings")
        ? "settings"
        : route.startsWith("/explore") || route.startsWith("/collections")
          ? "explore"
          : "print";
  const jobId = route.startsWith("/job/") ? route.slice(5) : null;

  return (
    <div className="app">
      {section === "print" && (
        <PrintView
          key={jobId ?? "new"}
          jobId={jobId}
          printers={printers}
          sharedLink={shared}
          onSharedConsumed={() => setShared(null)}
          openJob={(id) => go(`/job/${id}`)}
          reset={() => go("/")}
        />
      )}
      {section === "explore" && route.startsWith("/explore/model") && (
        <ModelPage
          key={route}
          url={new URLSearchParams(route.split("?")[1] ?? "").get("u") ?? ""}
          printers={printers}
          openJob={(id) => go(`/job/${id}`)}
        />
      )}
      {section === "explore" && !route.startsWith("/explore/model") && (
        <CollectionsView
          key={route}
          id={route.match(/^\/(?:explore\/c|collections)\/([^/?]+)/)?.[1] ?? null}
          openModel={(url) => go(`/explore/model?u=${encodeURIComponent(url)}`)}
        />
      )}
      {section === "printers" && /^\/printers\/[^/]+\/sensors/.test(route) && <SensorsView printers={printers} id={route.split("/")[2]} />}
      {section === "printers" && !/^\/printers\/[^/]+\/sensors/.test(route) && <PrintersView printers={printers} reload={loadPrinters} />}
      {section === "history" && route.startsWith("/history/stats") && <StatsView printers={printers} />}
      {section === "history" && !route.startsWith("/history/stats") && <HistoryView printers={printers} openJob={(id) => go(`/job/${id}`)} />}
      {section === "settings" && <SettingsView
          section={route.replace(/^\/settings\/?/, "")}
          printers={printers}
          reloadPrinters={loadPrinters}
          passwordFromEnv={auth.password_from_env}
          ingressUser={auth.ingress ? (auth.user ?? "Home Assistant") : null}
          passwordSet={auth.admin_password_set ?? true}
          onSignedOut={checkAuth}
        />}

      {info && ((section === "print" && !jobId) || route === "/settings") && (
        <p className="app-version">
          {tr("AirSlice {0} alpha{1}", [info.version, info.slicer.available && info.slicer.version ? tr(" · OrcaSlicer {0}", [info.slicer.version]) : ""])}
        </p>
      )}

      <div className="tabbar">
        <nav aria-label={tr("Sections")}>
          {(
            [
              ["print", "/", tr("Print")],
              ["explore", "/explore", tr("Explore")],
              ["printers", "/printers", tr("Printers")],
              ["history", "/history", tr("History")],
              ["settings", "/settings", tr("Settings")],
            ] as const
          ).map(([key, href, label]) => (
            <a key={key} className="tab" href={`#${href}`} aria-current={section === key ? "page" : undefined}>
              {Icon[key]}
              {label}
            </a>
          ))}
        </nav>
      </div>
      <ToastHost />
    </div>
  );
}
