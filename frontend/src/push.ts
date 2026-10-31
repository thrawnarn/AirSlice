import { api, inHomeAssistant } from "./api";
import { lang, tr } from "./i18n";
import type { NotifySettings } from "./types";

const ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
const standalone = window.matchMedia?.("(display-mode: standalone)").matches || (navigator as { standalone?: boolean }).standalone === true;

/** Why this browser can't take notifications here, or null when it can. */
export function pushBlocker(): string | null {
  if (inHomeAssistant) return tr("Browser notifications don't work inside Home Assistant. Open AirSlice at its own address to turn them on.");
  if (!window.isSecureContext) return tr("Browser notifications need AirSlice on an https address.");
  if (ios && !standalone) return tr("On iPhone and iPad, add AirSlice to the Home Screen first (Share, then Add to Home Screen) and open it from there.");
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return tr("This browser can't show notifications.");
  if (Notification.permission === "denied") return tr("Notifications are blocked for AirSlice in this browser's settings.");
  return null;
}

/** This browser's subscription endpoint, if it has one. */
export async function currentEndpoint(): Promise<string | null> {
  if (pushBlocker()) return null;
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    return (await reg?.pushManager.getSubscription())?.endpoint ?? null;
  } catch {
    return null;
  }
}

function key(b64: string): Uint8Array<ArrayBuffer> {
  const s = atob(b64.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (b64.length % 4)) % 4));
  const out = new Uint8Array(new ArrayBuffer(s.length));
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

function deviceName(): string {
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua) || (ios && !/iPhone/.test(ua))) return "iPad";
  if (/Android/.test(ua)) return "Android";
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "";
  const os = /Mac OS X/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
  return [os, browser].filter(Boolean).join(" · ") || tr("This browser");
}

/** Ask for permission, subscribe and register this browser as a notification. */
export async function enablePush(): Promise<{ settings: NotifySettings; endpoint: string }> {
  // First, while the tap still counts: Safari only asks from a user gesture.
  const permission = await Notification.requestPermission();
  if (permission !== "granted") throw new Error(tr("Notifications weren't allowed."));
  const reg = (await navigator.serviceWorker.getRegistration()) ?? (await navigator.serviceWorker.register("./sw.js"));
  await navigator.serviceWorker.ready;
  const { public_key } = await api.get<{ public_key: string }>("settings/notify/webpush");
  let sub = await reg.pushManager.getSubscription();
  if (!sub) sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key(public_key) });
  const json = sub.toJSON();
  const settings = await api.post<NotifySettings>("settings/notify/webpush", { endpoint: sub.endpoint, keys: json.keys, name: deviceName(), language: lang });
  return { settings, endpoint: sub.endpoint };
}
