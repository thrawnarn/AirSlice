# AirSlice

Self-hosted bridge that lets a Bambu Lab printer stay offline (LAN-only + Developer Mode)
while the owner still prints from their phone: paste a MakerWorld/Printables link →
pick profile/file and plate → slice with OrcaSlicer on the server → match filament to
AMS trays → upload over FTPS → start over MQTT. Runs as a Docker container or a Home
Assistant add-on. Typical setup: Home Assistant or Docker at home, the phone reaching it
over a VPN or a reverse proxy.

## Layout

- `backend/` — Rust (axum 0.8, tokio, rustls, rumqttc, reqwest 0.13, zip, quick-xml)
  - `main.rs` routing, `BASE_PATH` sub-path, ingress middleware, gzip, static files
  - `api/` HTTP handlers (`auth`, `printers`, `jobs`, `settings`), all under `/api`
  - `jobs.rs` pipeline: download → inspect → slice → (queue) → upload → start; queue,
    favourites, reprint, slice-all
  - `printer/` MQTT connection + status (`mod.rs`, `status.rs`), implicit FTPS client
    (`ftp.rs`), AMS mapping (`mapping.rs`)
  - `sources/` MakerWorld (via api.bambulab.com) and Printables (GraphQL) importers
  - `slicer.rs` OrcaSlicer CLI + profile index (resolves `inherits`), tray → filament profile
  - `threemf.rs` 3MF inspection (plates, thumbnails, slice_info); `preview.rs` plate geometry
  - `store.rs` state in memory; settings/printers/keys in `$DATA_DIR/state.json` (atomic
    writes, 0600), job history in SQLite `$DATA_DIR/airslice.db` (rusqlite bundled, table
    `jobs(id, created_at, data JSON)`, WAL + checkpoint after each write so HA's live
    file copy is complete). `update` diffs and writes only changed job rows, and
    state.json only when it changed. `Data::jobs` is `skip_serializing`: read from
    state.json once to move pre-alpha history over
  - `auth.rs` admin password (argon2), session cookie, API keys (`as_` prefix, sha256-stored),
    Home Assistant ingress trust (`INGRESS_TRUSTED_PEER`)
- `frontend/` — React 19 + TypeScript + Vite PWA, no router lib (hash routes), no UI kit.
  `PlatePreview.tsx` is three.js, lazy-loaded. Styles in one `styles.css` with tokens.
- `airslice/` — Home Assistant add-on (config.yaml, run.sh, DOCS.md, CHANGELOG.md). The add-on
  pulls the GHCR image (`image:` in config.yaml), so HA backups don't carry it; the image's
  `docker/entrypoint.sh` runs `run.sh` as root under the Supervisor (SUPERVISOR_TOKEN +
  /data/options.json) and drops to uid 1000 otherwise
- `repository.yaml` — makes the repo an HA add-on repository
- `.github/workflows/image.yml` — builds amd64/arm64 image to GHCR on tag `v*`
- `Dockerfile` — frontend + backend + extracted OrcaSlicer AppImage (default 2.4.2)
- `tools/fake-printer/` — simulated printer for testing without hardware (see below)

## Commands

```bash
cd backend && cargo test            # 96 unit tests
cd backend && cargo clippy --all-targets   # keep at zero warnings
cd frontend && npx tsc -b && npm run build
cd frontend && npm run dev          # proxies /api to $AIRSLICE_BACKEND or 127.0.0.1:8080

# run locally (slicing needs OrcaSlicer; everything else works without it)
cd backend && DATA_DIR=./data STATIC_DIR=../frontend/dist ORCA_SLICER_PATH=/path/to/AppRun cargo run
```

## Testing without a printer

```bash
pip install -r tools/fake-printer/requirements.txt
sudo python3 tools/fake-printer/fake_printer.py      # port 990 needs root on Linux
# FTP_MODE=clear simulates the A1 series (no encrypted data channel)
```

Add printer in the app: host `127.0.0.1`, serial `01P00A123456789`, access code
`12345678`, profile `Bambu Lab P1S 0.4 nozzle`. It reports 3 loaded AMS trays
(PLA white with RFID `GFA00`, PLA black, PETG red), records uploads and MQTT requests
in `tools/fake-printer/out/`, goes PREPARE → RUNNING on `project_file`, and enforces
vsftpd-style TLS session reuse on the FTPS data channel.

Fixtures in `tools/fake-printer/fixtures/`: `benchy.gcode.3mf` (pre-sliced, 2 filaments),
`gears.3mf` (2 plates, 3 colours, Bambu-style sub-model layout), `gears2.gcode.3mf`
(pre-sliced, 2 plates), `tpu.gcode.3mf` (needs filament that isn't loaded), `cube.stl`.
Pre-sliced files skip OrcaSlicer, so the whole print flow can be tested without it.

## Verified vs not yet verified

Verified against a real P1S (firmware 01.10, LAN Only + Developer Mode):
MQTT status/control, FTPS upload (vsftpd session reuse), print start, following a
print to the end (2 h 32 min and 13 min prints, 2026-10-30), HMS codes, AMS trays.
The image's OrcaSlicer 2.4.2 slices live MakerWorld projects (project settings,
printer profiles, copies). Live MakerWorld API: resolve, token download, collections.
Also verified here: auth/keys/ingress, queue/groups, favourites/reprint, preview
geometry, notifications (Telegram/ntfy against mocks), UI flows (Playwright
screenshots).

**Not run against the real thing yet** (expect fixes here first):
- the Thingiverse API (built from its public docs, tested against a mock)
- the live Printables API beyond what the fixtures cover
- other printer families: A1/A1 mini (`PROT C` FTP, only the fake printer), X1 (camera
  is RTSP, not supported), H2 (`ftp:///` url)

## Protocol notes (learned the hard way, keep)

- Printer certs are signed by Bambu's CA with the serial as CN; we accept any cert
  (`tls.rs`) but still verify handshake signatures. The cert is X.509 **v1**, which
  webpki rejects (`UnsupportedCertVersion`), so `tls.rs` reads the key from the cert
  itself instead of using rustls' `verify_tls1x_signature`. The fake printer serves a
  checked-in v1 cert (`fixtures/printer-cert.pem`; OpenSSL 3 can only make v3).
- FTPS: implicit TLS on 990, user `bblp`. X1/P1/P2/H2 (vsftpd) require the data channel
  to resume the control session → one rustls config per upload with session-id
  resumption. A1/A1 mini need `PROT C`. `Auto` mode tries one then the other. Ignore the
  address in the PASV reply. Send TLS close_notify before reading the 226.
- `project_file`: `url` = `file:///sdcard/<name>` (X1/P1/A1) or `ftp:///<name>` (H2), ids
  "0" — `ftp://<name>` (no third slash) makes a P1S take the task name and stay IDLE
  without any error (found 2026-10-29 on a P1S). The printer answers commands
  with `result`/`reason`; AirSlice logs them. With the correct url, a P1S
  (firmware with MQTT command verification) answered with HMS 0500_0500_0001_0007
  "MQTT command verification failed": unsigned commands only work with LAN Only +
  Developer Mode on the printer (HA's bambu_lab shows `hybrid_mqtt_blocks_control`).
  `start_print` watches `hms` and explains that code. Flat `ams_mapping` uses -1 for external spool, details in
  `ams_mapping2` (`ams_id` 255 for external on single-nozzle). `task_id` must fit int32.
- Remote file name = title (+ plate) + `_<6 hex of job id + output mtime>`: the
  printer only reports name and size, and "already on the printer" (skip upload) must
  mean this very slice. `print` holds a per-job claim (`STARTING`) until the job is
  Uploading/Waiting; one `wait_and_send` loop per job (`WAITING`).
- `JobState::in_use()` = busy or Started: slice/reslice/enqueue/slice_all refuse it and
  the history limit never drops it (`is_busy` alone is what a restart interrupts).
- Never send `project_file` while the printer is busy (A1 mini cancels the running job).
- Connection test (`PrinterManager::commands_refused`): re-sends the chamber light's
  current mode and watches 4 s for HMS 0500_0500_0001_0007 (Developer Mode off).
  Unverified against a refusing printer; it only ever adds a warning.
- MakerWorld download: design → `modelId` (alphanumeric) → `GET
  api.bambulab.com/v1/iot-service/api/user/profile/{profileId}?model_id=…` with Bearer
  token → presigned URL, fetch as-is (re-encoding breaks the signature). HTTP 418 = CAPTCHA.
- Thingiverse (`sources/thingiverse.rs`): anonymous downloads and the API answer 403
  from a server (checked 2026-10-29), so it needs an app token from
  thingiverse.com/developers (`Data::thingiverse_token`). `GET /things/{id}`,
  `/things/{id}/files`, `/files/{id}/download` (redirects to the file host; reqwest
  drops the token on the cross-host redirect). Tested against a mock only
  (`AIRSLICE_THINGIVERSE_API`); the real API is unverified until someone has a token.
- MakerWorld collections (api.bambulab.com/v1/design-service, Bearer token; found
  2026-10-30 from makerworld.com's JS): `GET my/favorites/listlite` (own: id, title,
  designCnt, designCover[]), `GET my/favorites/v2/like` (liked, with creator), `GET
  my/favorites/download/designs`, `GET favorites/{id}/designs?limit&offset` (hits: id,
  title, cover, designCreator.name, printCount; total). makerworld.com itself is behind
  a Cloudflare challenge; the API isn't. Pictures (`*.bblmw.com`): originals come as
  `application/octet-stream` + nosniff (browsers refuse them) and headless Chromium gets
  a Cloudflare page, so `makerworld::thumb` adds `?x-oss-process=image/resize,w_480/
  format,webp` and the browser loads them via `GET api/makerworld/image?u=` (bblmw.com
  only, cached in `data/cache/makerworld`, trimmed hourly by `jobs::trim_cache` to
  50 MB / 30 days unused, LRU by mtime which a hit refreshes; backup_exclude
  `cache/*`); `remoteImg()` in ui.tsx.
- Model details (`ResolvedModel::details`, model page / `ModelPanel` in ModelView.tsx):
  MakerWorld design JSON (`summaryTranslated`/`summary` + designer footer HTML,
  `designExtension.design_pictures`/`real_pictures`, tags, categories, counts, license,
  createTime; per profile `prediction`/`weight` from the design's own instance copy);
  Printables a separate `ModelDetails` GraphQL query. HTML → `sources::html::to_blocks`
  (p/h/li text blocks); the frontend renders text only.
- Printables: `POST api.printables.com/graphql/`, `getDownloadLink(id, printId, fileType,
  source: model_detail)`.
- Bambu Studio projects carry values OrcaSlicer's CLI range check refuses (BS 2.8:
  `raft_first_layer_expansion` -1, `wall/sparse_infill/solid_infill_filament` 0,
  `tree_support_wall_count` -1). Orca prints `key: value not in range [..]` and exits
  -18; `slicer.rs` drops exactly those keys (`threemf::drop_settings`) and retries once.
  OrcaSlicer 2.3.1 segfaulted on BS 2.8 projects even after that; 2.4.2 slices them.
- OrcaSlicer 2.4 `result.json` says `"error_string": "Success."` with `return_code` 0 —
  only the return code means failure.
- Copies: `--repetitions N --arrange 1` works on 3MF projects (all objects of the plate
  are repeated), but fails with -2 on a bare STL in 2.4.2 → `slicer.rs` first exports the
  mesh to a project (`--export-3mf` without `--slice`, outdir must exist), then repeats.
  Copies that don't fit are silently put on another plate: `jobs::check_copies` compares
  the plate's `<object>` count in slice_info with plate object_count × copies.
- Loading filaments replaces the project's filaments (needs one per slot); colour comes
  from `filament_colour` in the written profile. A filament with `<plate>_temp` 0 (PETG:
  `cool_plate_temp`) makes Orca exit -61 "Cool Plate does not support filament";
  `ProfileIndex::plate_for` picks `curr_bed_type` (process.json, or the 3MF's project
  settings) up front. Orca's default plate is the Cool Plate.
- Tray MQTT: `ams_filament_setting` (ams_id, tray_id + slot_id, tray_info_idx = profile
  `filament_id` e.g. GFL99, tray_color RRGGBBAA, nozzle_temp_min/max, tray_type;
  external = ams 255 / tray 254), `ams_get_rfid` (ams_id, slot_id; no external).
  The fake printer applies both.
- Build plates (`curr_bed_type` values): `Textured PEI Plate`, `Cool Plate`,
  `Supertack Plate` (Orca also accepts "SuperTack Plate" and saves it lower-t),
  `Textured Cool Plate`, `Engineering Plate`, `High Temp Plate`. An unknown value falls
  back to the Cool Plate. `slicer::BUILD_PLATES` maps each to its filament temp key;
  printers store `plates` + `default_plate`; the chosen plate goes into process.json or
  the 3MF's project settings.
- Offline printers: `jobs::print` puts the job in `Waiting` (`waiting_until`,
  `pending_print`) and `wait_and_send` polls every 15 s, waking the MQTT reconnect
  (`PrinterManager::wake`). It must wait for a *fresh* report after reconnecting
  (last_update ≥ connect time) and re-request pushall: a just-booted broker can drop
  the first one. Waiting jobs survive restarts (`recover`). Settings: `ConnectionSettings`
  in state.json, `PUT /api/settings/connection`. `NOT_CONNECTED` means nothing was sent,
  so the start command is safe to retry. The fake printer takes `FAIL_UPLOADS=N`.
- Slice progress: `--pipe <path>` makes OrcaSlicer write JSON lines
  (`total_percent`, `message`); a plain file works, `slicer.rs` reads it every 0.5 s.
  Progress is kept in memory (`AppState::progress`), not in state.json.
- Colour painting: triangles carry `paint_color` (Bambu) / `slic3rpe:mmu_segmentation`
  (Prusa), a TriangleSelector hex string read from the end (low 2 bits = split sides;
  leaf state = bits 2–3, 3 = extended: next nibble + 3; state n = filament n).
  `preview.rs` gives each triangle its most common painted filament.
- Preview size: meshes are simplified by vertex clustering to ~250 k triangles per plate
  (shared by detail, ≥ 2 k per object), positions packed as u16 over each object's box
  (`origin` + q·`step`), indices u16 when they fit. T-rex plate: 2 M → 233 k triangles,
  21 MB → 1.7 MB gzipped, edges within 0.4 mm. Cache files are `preview-v3-…`; bump the
  version when the format changes. Model XML is streamed out of the zip (`read_model`,
  one attribute pass per vertex/triangle) and only the chosen plate's sub-models are
  read. Meshes over 1.5 M points are merged into a 1200-cell grid as soon as their
  vertices are read (`merge_points`), so triangles map straight onto merged points;
  the cap is 60 M triangles read. Generated plates in the release build: 6.5 M
  triangles 5 s / ~310 MB, 20 M (1.4 GB XML) 13 s / ~390 MB, 1.4 MB sent. Preview
  failures are logged (`preview failed`).
- Headless Chromium on the Alpine HA SSH add-on has no WebGL (not even SwiftShader);
  PlatePreview catches the failure. For screenshots with the 3D preview run Chromium
  headful under `xvfb-run` (apk `xvfb-run mesa-gl mesa-egl`, `LIBGL_ALWAYS_SOFTWARE=1`,
  playwright `headless: false`, `--hide-scrollbars`). Emoji/arrows need apk
  `font-noto-emoji font-dejavu`. README screenshots live in `docs/screenshots/`.
- The service worker must leave cross-origin requests alone: it runs under the page's
  CSP (`connect-src 'self'`), so fetching a Printables/Thingiverse picture there fails
  and the cached page came back instead (broken covers in the installed app).
- Leaving objects out of a plate: remove only the plate's `<model_instance>` entries in
  model_settings.config (`threemf::keep_plate_objects`). Removing the `<item>` from the
  3dmodel build makes OrcaSlicer fail to load the file (-6). Copies + arrange keep the
  removed objects out.
- Following prints: `jobs::watch_prints` (every 5 s) matches the printer's
  `subtask_name` to `PrintStart::task_name()`; FINISH → finished, FAILED with
  0x0300400C → cancelled, other FAILED → failed, another task running → unknown.
  Only the outcome is stored; live progress is read from the printer per request.
- Notifications are a list (`NotifySettings::targets`, each `NotifyTarget` has channel,
  recipient, events, photos); the HA address/token and the tap link are shared.
  `NotifySettings::migrate` (run in `Store::open`) turns the pre-alpha single setting into
  target "1"; the old fields are read but never written. Tokens are kept per target id.
  The camera picture is taken once per event and shared. Test: `POST
  settings/notify/test?target=<id>` with the settings on screen.
- Notification channels (`notify.rs`): `ha`, `telegram` (sendPhoto multipart / sendMessage;
  `AIRSLICE_TELEGRAM_API` points tests at a mock), `ntfy` (PUT the JPEG with
  Title/Message/Filename headers, non-ASCII as RFC 2047 base64). Tokens never go back
  to the browser (`has_*_token`); empty means keep. `first_layer` fires once when the
  printer reports layer ≥ 2 (`PrintStart::first_layer_done`). HA can only attach
  `/api/camera_proxy/<entity>` (HA camera source). Test Telegram against a mock or a
  private chat, never a group.
- Power plug (`power.rs`): `Printer::power_switch` (HA switch) + `power_sensor`/
  `energy_sensor` found once via `/api/template` `device_entities(device_id(switch))` by
  device_class. Uses the notifications' HA endpoint. `jobs::print` to an offline printer
  turns an off plug on, then the normal Waiting flow. Auto-off (`power_off_after_min`)
  is armed only by `events::print_ended` (`power::arm`), counts from idle + nozzle
  < 50 °C + nothing queued/waiting/sending, checked every 60 s; a new print disarms.
  Energy: meter read at start (`PrintStart::energy_start`) and end → `energy_kwh`. Test
  without touching real plugs: run with `env -u SUPERVISOR_TOKEN` and `ha_url` pointing
  at a local mock of /api/states, /api/services/switch/*, /api/template.
- Notification texts: `i18n.rs` (English key → `SV` table, `t(lang, "…{0}", &[..])`),
  built per target by a `notify::Text` closure; `NotifyTarget::language` "" = HA's
  `/api/config` language. A test checks every `t(l, "…")` in events/notify/settings has
  Swedish. Times: `jiff` with bundled tzdb (image has no tzdata), zone from `TZ` (the
  Supervisor sets it) else HA's `time_zone`. Pictures (`notify::Pictures`): camera
  snapshot and/or plate PNG; Telegram >1 = sendMediaGroup, ntfy the first, HA one URL
  (camera_proxy for an HA camera, else a copy in `/media/airslice` → `/media/local/…`,
  add-on `map: media:rw`). Test Telegram with `AIRSLICE_TELEGRAM_API` → a local mock.
- Browser notifications (`webpush.rs`, channel `webpush`): RFC 8291 aes128gcm + VAPID
  (RFC 8292) with ring (already there for rustls), key in `Data::webpush_key`. The
  browser subscribes via `POST settings/notify/webpush` (frontend `push.ts`); its
  endpoint/keys are only set there (`keep_secrets` copies them, new entries can't
  bring them; p256dh/auth never go to the browser). 404/410 turns the entry off.
  Payload `{title, body, url, image}`, relative to the app; `sw.js` shows it and opens
  `url` on tap. Pictures: in memory 1 h at the public `GET api/notify/picture/{id}`
  (unguessable id). No service worker inside HA ingress, so no push there; iOS only
  from the Home Screen app, without images. Verified against Python `http_ece` and a
  local HTTPS receiver (backend with `SSL_CERT_FILE`), not yet a real push service.
- Notifications: Home Assistant notify services, in the add-on via
  `http://supervisor/core/api` with SUPERVISOR_TOKEN (`homeassistant_api: true`), the
  panel link from `/addons/self/info` (`hassio_api: true`). `events.rs` holds the hooks.
- Spoolman: spools match trays by `extra.tag` = AMS `tray_uuid` (upper case, quotes
  stripped), else manual links in state.json. Price per g: spool price, else filament
  price, per initial_weight/filament weight. Booking (`PUT /api/v1/spool/{id}/use`) is
  off by default, since other tools may already book usage.
- Job files: `jobs::clean_files_hourly` removes job folders older than
  `storage.keep_files_days` (default 30), keeping favourites/queued/running; the job
  gets `files_removed` and "Print again" downloads it again (uploads can't). `recover`
  also sets `files_removed` on jobs whose input file is gone (a moved/restored data
  folder), and `reprint` checks the file on disk too.
- Next plate (`jobs::next_plate`): jobs of the same file = same source, model_url,
  file.id and file.name; a plate is taken when a job printed it, has it queued/sending,
  or is an `auto_queue` copy still Ready/Slicing (its plate is in `auto_slice`). Shown as
  `next_plate` on a sent job's view; `GET jobs/next-plate` is the start-page hint (only
  with an empty queue, newest `print.started_at`); `POST jobs/{id}/queue-next-plate`
  slices with the printed job's settings/trays/bed and auto-queues.
- Camera (`camera.rs`): P1/A1 = TLS to port 6000, 80-byte login (u32 LE 0x40, 0x3000,
  0, 0, then "bblp" and the access code each padded to 32 bytes), frames = 16-byte
  header (JPEG size LE first) + JPEG; X1 is RTSP (not supported). Or a Home Assistant
  camera via `/api/camera_proxy/<entity>` (same endpoint/token as notifications, so it
  works outside the add-on with a URL + token). The printer allows one camera client,
  so a printer whose camera HA already reads uses that HA camera. Pictures cached 2 s per
  printer+source. Result photo at print end → `data/photos/<job>.jpg` (outside the job
  folder, so file cleanup keeps it). The fake printer serves `fixtures/camera.jpg` on 6000.
- Layer view (`gcode.rs`): the plate's G-code is copied out of the sliced 3MF into the
  job folder (`layers-<plate>-<mtime>-<size>.gcode`) and indexed once (`layers-v3-….json`:
  byte range + machine state per `; CHANGE_LAYER`); a layer is read by seeking. Orca
  for Bambu uses arc fitting (G2/G3 with I/J, also full circles), G91 in macros, M83.
  Extrusion without X/Y movement is a purge at the chute (x −48 on A1/P1) and is
  skipped, or it stretches the framing. T255/T1000 are macro internals. Trex 2.2 M
  lines: index 0.8 s (release), a layer ~20 ms / ~100 KB gzipped.
- Quick settings (`SliceTweaks` → Orca keys `layer_height`, `wall_loops`,
  `sparse_infill_density` "40%", `enable_support` + `support_type` "tree(auto)",
  `brim_type` outer_only/no_brim + `brim_width`) are written like the build plate: into
  process.json, or the 3MF's project settings (`edit_settings`). Checked in the G-code
  header for both paths. Presets live in `Data::slice_presets` (None = defaults).
  `fixtures/gears.3mf` and the "painted" test file don't slice in real Orca; use a real
  MakerWorld project.
- AirSlice must work without Home Assistant: HA features (notifications, HA camera)
  are optional and explain what to set up when HA isn't reachable.
- HMS texts (`printer/hms.rs`): Bambu's `e.bambulab.com/query.php?lang=en|sv`
  (`data.device_hms.<lang>[{ecode, intro}]`), cached `data/cache/hms_<lang>.json`,
  refreshed weekly; `Problem.texts`. Wiki pages aren't derivable (family path, AMS codes
  use a base code: 0700_2300 → 0700_2000), so `printer/hms_wiki.json` (from ha-bambulab,
  MIT) maps code → page, else the HMS overview. Hidden messages: per device in
  localStorage `hms-hidden:<printer>`, dropped once the code is gone.
- The fake printer takes `PRINT_SECONDS=N` (finish after N s) and `HMS=attr:code,…`
  (hex) and reports a stop like a P1. HMS severity is the code's third group
  (0001 fatal, 0002 serious, 0003 common); `StatusView.problems` carries text + wiki URL.
- Running the image's OrcaSlicer outside Docker (e.g. on the Alpine HA SSH add-on):
  unpack the GHCR layers, then start `bin/orca-slicer` through the image's
  `ld-linux-x86-64.so.2 --library-path …` (no chroot/proot needed).

## Security notes

- Home Assistant entity ids and notify service names go into HA API paths and a
  template: always `notify::is_entity_id(id, domain)` / `is_object_id` (lower-case,
  digits, `_`), on save and where used. Stored HA/ntfy tokens only go to the stored
  address (`keep_secrets`, also in `notify_services`).
- Login: one password check at a time; 10 wrong within 10 min → 60 s lockout (global).
- First password (`/auth/setup`, standalone only) only from a loopback, private,
  link-local, ULA or CGNAT peer (`api::auth::is_local`); else ADMIN_PASSWORD.
- Diagnostics (`diagnostics.rs`, `GET api/diagnostics`, Settings → About): a second
  tracing fmt layer keeps the last 300 lines in memory; the report is redacted with
  every access code, serial (→ last 4), token, ntfy topic, chat id and push key from
  the state. Add new secrets to `diagnostics::secrets`.

- Ingress trust = TCP peer `INGRESS_TRUSTED_PEER` (172.30.32.2, the Supervisor) AND an
  `X-Ingress-Path` header (`auth::mark_ingress`); never trust headers alone. The
  X-Remote-User headers are display only.
- Through ingress the pages share Home Assistant's origin (where HA keeps its tokens):
  never render remote text as HTML, never serve uploads as HTML/SVG, keep the CSP in
  `security_headers` (scripts 'self' only; styles need 'unsafe-inline' for React style
  attributes; images https: for model covers).
- Add-on: `/auth/setup` is refused (first password only via ingress Settings → Access or
  the add-on option), so an opened port can't be claimed.

## Conventions

- Notification text is translated the same way on the server (`i18n.rs`); job log and
  error messages stay English.
- UI text is translated with `tr("English text", [args])` (`frontend/src/i18n.ts`);
  the English text is the key and Swedish lives in `sv.ts` ({0}, {1} placeholders).
  Wrap every new UI string and add its Swedish; untranslated keys fall back to English.
  Don't build sentences from word pieces (plurals: separate keys). Use `locale` from
  i18n for dates/numbers. Server messages (errors, job log) are English only.

- UI copy: plain, sentence case, active verbs; errors say what happened and what to do.
- Design: graphite + anodized aluminium + PEI-gold accent; filament colours are the only
  bright colour. Light and dark themes via tokens in `styles.css`. Mobile-first (iPhone).
- The queue never starts a print by itself; a person confirms the plate is clear.
- Queue groups: `jobs::queue_group` = the printer profile (`machine_profile`) of the
  item's printer; positions, moves, `start_next` and the card's count are per group.
  Printer setup lives in Settings → Printers (`PrinterSettings`, `#/settings/printers/<id|new>`
  opens `PrinterForm`); the Printers tab is live cards only.
  The printer card shows only the group's next print (Start → `start-next`); ordering and
  removing live in `QueueBoard` on the Print page.
  `POST queue/{id}/start {printer_id}` (`jobs::start_queued`) moves a print to another
  printer with the same profile (bed plate must be among its plates; chosen trays are
  dropped and re-matched), then prints. `POST queue/{id}/reslice {printer_id}`
  (`jobs::reslice_queued`) slices it again for a printer of another type, in place
  (`auto_queue`, cleared once queued or failed): `SliceOutcome::options` keeps what
  was asked (plate, objects, settings, tweaks, transform); trays, process/filament
  profiles and a plate the printer lacks are dropped, `project` settings become auto.
  Pre-sliced files refuse. At print end `events::print_ended` sends the `queue` event
  for the finished printer's group ("X is free — N prints waiting").
- Release: bump version in `airslice/config.yaml` (it is also the image tag),
  `backend/Cargo.toml`, `frontend/package.json`; add to `airslice/CHANGELOG.md`.
- GitHub: https://github.com/thrawnarn/AirSlice (private). Image: `ghcr.io/thrawnarn/airslice`
  (lowercase), referenced as `image:` in `airslice/config.yaml`; the GHCR package must be public for
  Home Assistant to pull it.
- Commit messages end with a `Co-Authored-By` line for Claude.
