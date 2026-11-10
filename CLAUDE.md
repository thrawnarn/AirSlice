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
- `card/` — `airslice-history-card` (Lit, plain JS): `npm test` in happy-dom, `npm run build` writes
  `custom_components/airslice/www/`, which is committed. `node_modules` is a symlink to a cache.
  `card.py` serves it and keeps one dashboard resource, `?v=` = hash of the built file.
- `custom_components/airslice/` — Home Assistant integration (HACS-style, not part of the
  add-on or image): reads `GET api/prints` every 60 s and on each `list_prints`, one
  sensor per printer, photos through a media source and an authenticated view. It stores
  nothing: AirSlice owns the history. `prints.py` is pure. `tests/ha` runs it on real HA.
- `repository.yaml` — makes the repo an HA add-on repository
- `.github/workflows/image.yml` — builds amd64/arm64 image to GHCR on tag `v*`
- `Dockerfile` — frontend + backend + extracted OrcaSlicer AppImage (default 2.4.2)
- `tools/fake-printer/` — simulated printer for testing without hardware (see below)

## Commands

```bash
cd backend && cargo test            # 124 unit tests
cd backend && cargo clippy --all-targets   # keep at zero warnings
pytest tests/ha                     # integration, needs pytest-homeassistant-custom-component
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
- `JobState::in_use()` = busy or Started: slice/reslice/enqueue/slice_all/delete refuse it and
  the history limit never drops it (`is_busy` alone is what a restart interrupts).
- Never send `project_file` while the printer is busy (A1 mini cancels the running job).
- Connection test (`PrinterManager::commands_refused`): re-sends the chamber light's
  current mode and watches 4 s for HMS 0500_0500_0001_0007 (Developer Mode off).
  Unverified against a refusing printer; it only ever adds a warning.
- MakerWorld download: `GET api.bambulab.com/v1/design-service/design/{id}` → the
  `instances[]` entry with this `profileId` → `GET
  /v1/design-service/instance/{instance id}/f3mf?type=download` with Bearer token →
  `{name, url}`, presigned, fetch as-is (re-encoding breaks the signature). HTTP 418 =
  CAPTCHA. The older `/v1/iot-service/api/user/profile/{profileId}?model_id={modelId}`
  answered 200 "success" with `url: ""` on 2026-11-04 (three models checked); it's only
  the fallback now. An empty string makes reqwest fail with `builder error`, before
  anything is sent.
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
- AMS drying: `ams_filament_drying` (ams_id, temp, cooling_temp ≥ 45 or it's ignored,
  duration in hours 1–24, mode 1 start / 0 stop, rotate_tray; from ha-bambulab). AMS 2
  Pro up to 65 °C, AMS HT 85 °C. The model comes from `{"info":{"command":"get_version"}}`
  → `info.module[].name` (`n3f/0`, `n3s/128`, `ams/0`, `ams_f1/0`), `dry_time` in the
  report is minutes left, `tray_temp`/`tray_time` the spool's own values. A unit feeding
  a print can't dry (`AmsUnit::in_print`); a print can't use a drying unit (`StatusView::drying_refusal`,
  checked in `jobs::send` before the upload), and matching avoids one when it can. A real P1S (firmware 01.10, 2026-11-01) answers
  the command with "success" and ignores it, as ha-bambulab #1448/#1738 report for P1S/P1P;
  only a changed report counts as started. Started on its screen, it reports `dry_time`
  and `info` 2023/2024 (bits 4–7 = 2). `ignores_drying_command` hides the controls on P1S/P1P;
  everywhere else they're unconfirmed and the sheet asks for a report on GitHub.
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
- Preview size: meshes go through `meshopt` edge collapse down to ~250 k triangles per plate (≥ 2 k per object), with
  one allowed error in mm for the whole plate, raised from 0.005 by ×1.5 until it fits (`simplify_plate`; 13 rounds ≈ 0.8 mm, then
  `simplify_sloppy` by share). Don't go back to vertex clustering or a budget by triangle count: averaging in a grid merged the two
  sides of thin walls (pits in a printed egg, holes in the t-rex's tail) and flattened ribs, and a dense thread starved a big
  wall beside it. Borders are locked when an object has several filaments, or gaps open between colours. `sanitize` runs first,
  since the C++ simplifier trusts indices. `weld` makes STL soup indexed, for the simplifier and for the normals.
  Positions packed as u16 over each object's box (`origin` + q·`step`), indices u16 when they fit.
  `meshNormals.ts` makes normals with a 35° crease, as `computeVertexNormals` rounds every edge on shared points.
  T-rex plate: 2 M → 170 k triangles in 5 s, 2 MB before gzip; 6.5 M generated 12 s / 475 MB, 20 M 26 s / 544 MB.
  Cache files are `preview-v4-…`; bump the version when the format or the simplifying changes. Model XML is streamed out of the zip (`read_model`,
  one attribute pass per vertex/triangle) and only the chosen plate's sub-models are
  read. Meshes over 1.5 M points are merged into a 1200-cell grid as soon as their
  vertices are read (`merge_points`), so triangles map straight onto merged points;
  the cap is 60 M triangles read. Preview
  failures are logged (`preview failed`).
- Headless Chromium on the Alpine HA SSH add-on has no WebGL (not even SwiftShader);
  PlatePreview catches the failure. For screenshots with the 3D preview run Chromium
  headful under `xvfb-run` (apk `xvfb-run mesa-gl mesa-egl`, `LIBGL_ALWAYS_SOFTWARE=1`,
  playwright `headless: false`, `--hide-scrollbars`). Emoji/arrows need apk
  `font-noto-emoji font-dejavu`. README screenshots live in `docs/screenshots/`.
- The service worker must leave cross-origin requests alone: it runs under the page's
  CSP (`connect-src 'self'`), so fetching a Printables/Thingiverse picture there fails
  and the cached page came back instead (broken covers in the installed app).
- More models on a plate (`Job::extras`, files in the job's `extras/`, `SliceOptions::extras`,
  `POST jobs/{id}/extras`, `…/extras/upload`, `…/extras/from-job`, `DELETE …/extras/{extra}`,
  `AddModelSheet` and `FileCard` in PrintView.tsx, `CollectionPicker`, `SliceOutcome::files` = each
  file's slots, which trays can make files share; picking another tray for one splits it with a new slice, `slicer.rs`
  `extra_meshes`/`combine`; checked on OrcaSlicer 2.4.2, 2026-11-04). Orca takes one 3MF
  plus any meshes, 3MF first; two 3MFs exit -4. So an added model always becomes meshes
  (`--export-stl`, files `stl/obj_<n>_…`, n = place in the 3dmodel's `<item>` order, which
  picks the plate's objects; loses colours and project settings; `--orient 1` only for bare meshes).
  Refused project settings (-18) stop `--export-stl` and the combined export too, so both retry like slicing does.
  Mesh base: `--load-filament-ids 1,2,…` gives each input a filament. It's refused with a 3MF (-2).
  Project base: `threemf::single_plate` (only the chosen plate, as plate 1: `--arrange`
  gathers all plates onto one, and a later plate gives no G-code; the others must lose
  `<item>`, `<object>` and `assemble_item`, or loading fails -6) and `pad_filaments`
  (extra `--load-filaments` segfault or abort with "invalid size" otherwise; list lengths can't
  tell per-filament keys when a project has 1–2 filaments, hence `filament_keys.rs`) →
  `--export-3mf --arrange 1` without filaments (they segfault here) → `set_object_slots`
  edits `extruder` by object name (`airslice-e<extra>_…`) → `--slice 1`. `--repetitions` fails
  on an export without slicing, and `--clone-objects` fails (-2) on a 3MF, so `single_plate` repeats `<item>`, `<model_instance>` and the assembly entry itself; it also gathers several plates (`SliceOptions::also_plates`). Overflow: Orca
  moves objects to another plate (`check_all_placed` counts), or -102 "unprintable area", which
  is also the prime tower growing off the bed (default y 220; cube + Benchy in two colours does it).
  `wipe_tower_x/y` in process.json made no difference and re-exporting a mesh project segfaults, so -102 retries once with
  `enable_prime_tower` 0, then says the models don't fit. The output has one plate: `slice.plate` is 1, `options.plate` is the file's plate.
- A plate's filaments (`PlateInfo::filaments`, `threemf::set_plate_filaments`): each object's and part's
  `extruder` plus feature filaments. Unknown (all shown) when any model is painted or
  `custom_gcode_per_layer` changes filament, which scanning meshes would be needed for. Trays of slots
  off the plate are still sent, so "next plate" keeps them.
- Several filaments on one tray: re-slicing with the same colours changes nothing, since Orca's CLI
  keeps the project's `flush_volumes_matrix` and still changes tool (t-rex plate: 132 g, 10.2 h,
  255 changes at any colours; zeroing the matrix alone 52 g, 8 h, 223 changes). `jobs::merged_slots` +
  `threemf::merge_filaments` move `extruder` values and layer changes to the first slot on that tray and
  zero the matrix between them. Painted triangles keep their slot (not remapped), so they change without
  purging. Colours chosen after slicing: another colour count re-slices (`POST jobs/{id}/slice-with-trays`,
  added models keep their slot's new tray); same count only updates `cost-with-trays`, as the matrix
  isn't recalculated for new colours anyway.
- Leaving objects out of a plate: remove only the plate's `<model_instance>` entries in
  model_settings.config (`threemf::keep_plate_objects`). Removing the `<item>` from the
  3dmodel build makes OrcaSlicer fail to load the file (-6). Copies + arrange keep the
  removed objects out.
- Following prints: `jobs::watch_prints` (every 5 s) matches the printer's
  `subtask_name` to `PrintStart::task_name()`; FINISH → finished, FAILED with
  0x0300400C → cancelled, other FAILED → failed, another task running → unknown.
  Only the outcome is stored; live progress is read from the printer per request.
- Print history (`prints.rs`): a job with a `print.outcome` is a print record, which
  `max_jobs` never drops (`jobs::is_print_record`). `GET api/prints` = those, by end time.
  `notice_external` (every 5 s, in `watch_prints`) makes a `Source::External` job when a
  connected printer runs a named task (RUNNING/PREPARE/PAUSE, not `print_type` system) and
  no Started job is on it; it waits while any job is Uploading/Starting, since our own
  print can show on the printer first. From there it's a normal Started job (notifications,
  auto-off, photo). No slice, so cost is machine time only and reprint refuses.
  `started_at` is worked back from percent/time left only when RUNNING/PAUSE at 5 %+
  (PREPARE can still report the previous print's numbers). `PrintStart::is_task` compares
  without file endings, since external names may keep theirs. Imports
  (`POST prints/import`, `tools/import-arkiv`): External + `files_removed`, id = hash of
  `key`, a `SliceOutcome` with `settings: "imported"` carrying the log's real usage (so no
  progress share), skipped when a job has the same task ended within 15 min. A downgrade
  can't read `source: external`, so going back to an older image needs those jobs removed.
- Waste (`gcode::waste`): flush + prime tower grams from the plate's G-code at print end,
  up to the last layer for a stopped print; part of the filament weight, not on top.
  Matches `pandan_print_waste.py` on real Bambu Studio multi-colour files (2026-11-02).
- Notifications are a list (`NotifySettings::targets`, each `NotifyTarget` has channel,
  recipient, events, photos); the HA address/token and the tap link are shared.
  `NotifySettings::migrate` (run in `Store::open`) turns the pre-alpha single setting into
  target "1"; the old fields are read but never written. Tokens are kept per target id.
  The camera picture is taken once per event and shared. Test: `POST
  settings/notify/test?target=<id>` with the settings on screen.
- A paused print isn't a failed job, so `attention.rs` watches every printer (5 s, pure `step`):
  PAUSE held 10 s, new serious/fatal HMS held 10 s, contact lost 5 min during a print and back
  (event `attention`). A real P1S out of filament sets PAUSE + HMS and leaves `print_error` 0, and it
  flips PAUSE/RUNNING within seconds while someone resumes, hence the hold and the 15 min per-code
  repeat limit. Pausing from AirSlice isn't reported (`paused_by_us`). To test: send `pause` to the fake
  printer's broker (plain MQTT on 1884) from another client, with `DRY_HMS` for the message.
- Filament check (`filament_check.rs`, `POST jobs/{id}/filament-check`; print page, queue card, start and
  queue notifications): grams left = Spoolman, else AMS `remain` %. A real P1S with the remaining estimate off
  (home_flag 0x80) reports `remain` -1 on every tray, so without Spoolman the amount is unknown and the UI says so.
  A short tray is `covered` when other AMS trays with the same type, colour and `tray_info_idx` hold the rest
  (beyond their own share) and the backup setting is on (`home_flag` 0x400, on for a real P1S). Backups across AMS
  units and to/from an AMS HT aren't confirmed on a printer; forum reports say across units works.
- `almost_done` (own toggle) fires once at 1–5 min left, RUNNING, layer ≥ 2 and 50 %+ (`PrintStart::almost_done_sent`).
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
  < 50 °C + nothing queued/waiting/sending + no AMS drying, checked every 60 s; a new
  print or the plug seen off (e.g. an HA automation got there first) disarms.
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
- MCP server (`mcp.rs`, `POST api/mcp`): stateless Streamable HTTP, JSON answers only (GET/DELETE 405),
  protocol versions 2025-06-18/2025-03-26/2024-11-05. Each tool is planned into a REST call (`plan`) and run through
  `api::router` with the caller's own key headers (`tower::oneshot`), so auth and validation are the handlers' own. API
  keys only (a cookie session is refused, since another page could call tools with it), and the Origin must match the Host.
  Ids for paths go through `path_id`. Print tools need `plate_is_clear: true` (the queue rule). `shape` shortens job
  lists and logs. A new tool needs an entry in `tools()` and in `plan()`; `every_tool_has_a_plan` checks both.
  `print_file` is the one native tool (calls `jobs::` directly): colours → trays with `colors::pick` (names incl. Swedish,
  several shades per name, CIELAB ΔE ≤ 35 or the tray's own nearest name; a material word filters), refuses a busy
  printer, checks colour count against the file and, for pre-sliced files, the slot's material (`mapping::base_type`).
  Uploads/earlier jobs get `auto` + `auto_print` and then `jobs::slice`, so they start once sliced; links go through
  `jobs::create` like quick-print. A link's file isn't known up front, so those checks only happen at slicing there.
- Commit messages end with a `Co-Authored-By` line for Claude.
