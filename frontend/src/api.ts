import { tr } from "./i18n";

// All requests are relative to the page, so the app works at "/" and
// behind a reverse proxy sub-path such as "/airslice/".

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

let onUnauthorized: (() => void) | null = null;
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = { method, credentials: "same-origin", headers: {} };
  if (body instanceof FormData) {
    init.body = body;
  } else if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)["Content-Type"] = "application/json";
  }
  let res: Response;
  try {
    res = await fetch(`api/${path}`, init);
  } catch {
    throw new ApiError(0, tr("Can't reach AirSlice. Check that you're on your home network or VPN."));
  }
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (!res.ok) {
    if (res.status === 401 && !path.startsWith("auth/")) onUnauthorized?.();
    const msg = (data as { error?: string } | null)?.error ?? tr("Request failed ({0})", [res.status]);
    throw new ApiError(res.status, msg);
  }
  return data as T;
}

export const api = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, body ?? {}),
  put: <T>(path: string, body: unknown) => request<T>("PUT", path, body),
  del: <T>(path: string) => request<T>("DELETE", path),
  upload: <T>(path: string, form: FormData) => request<T>("POST", path, form),
  /** A plain-text answer. */
  text: async (path: string) => {
    const res = await fetch(`api/${path}`, { credentials: "same-origin" }).catch(() => {
      throw new ApiError(0, tr("Can't reach AirSlice. Check that you're on your home network or VPN."));
    });
    if (!res.ok) throw new ApiError(res.status, tr("Request failed ({0})", [res.status]));
    return res.text();
  },
};

/** Pull a supported model link out of shared text ("Look at this! https://…"). */
export function findModelLink(text: string | null | undefined): string | null {
  if (!text) return null;
  const match = text.match(/https?:\/\/[^\s"'<>]*(makerworld\.[a-z.]+|printables\.com|thingiverse\.com)[^\s"'<>]*/i);
  if (match) return match[0];
  const bare = text.match(/(makerworld\.[a-z.]+|(www\.)?printables\.com|(www\.)?thingiverse\.com)\/[^\s"'<>]+/i);
  return bare ? `https://${bare[0]}` : null;
}

/** Running inside Home Assistant (add-on ingress panel). */
export const inHomeAssistant = window.location.pathname.includes("/api/hassio_ingress/");
