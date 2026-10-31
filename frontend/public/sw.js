// Minimal service worker: makes the app installable and keeps the shell
// available when the server is briefly unreachable. API calls always go
// to the network.
const CACHE = "airslice-v1";

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE).then((c) => c.addAll(["./", "./manifest.webmanifest", "./icon-192.png"])));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))),
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  const url = new URL(req.url);
  // Pictures from Printables and Thingiverse load straight from the browser:
  // the page's connect-src 'self' would block fetching them from here.
  if (req.method !== "GET" || url.origin !== location.origin || url.pathname.includes("/api/")) return;
  // Network first, cache as fallback.
  event.respondWith(
    fetch(req)
      .then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(req, copy));
        }
        return res;
      })
      .catch(() => caches.match(req).then((hit) => hit || caches.match("./"))),
  );
});

// Browser notifications (Web Push). The server sends {title, body, url,
// image}; url and image are relative to the app.
self.addEventListener("push", (event) => {
  let d = {};
  try {
    d = event.data ? event.data.json() : {};
  } catch {
    d = { body: event.data ? event.data.text() : "" };
  }
  const at = (p) => new URL(p, self.registration.scope).href;
  const options = { body: d.body || "", icon: at("icon-192.png"), badge: at("icon-192.png"), data: { url: at(d.url || "./") } };
  if (d.image) options.image = at(d.image);
  event.waitUntil(self.registration.showNotification(d.title || "AirSlice", options));
});

// A tap opens the job: in an open AirSlice window when there is one.
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = (event.notification.data && event.notification.data.url) || self.registration.scope;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((list) => {
      const open = list.find((c) => c.url.startsWith(self.registration.scope));
      if (open) return open.focus().then((c) => (c && c.navigate ? c.navigate(url) : c)).catch(() => self.clients.openWindow(url));
      return self.clients.openWindow(url);
    }),
  );
});
