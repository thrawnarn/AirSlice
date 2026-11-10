# Changelog

## 0.4.1

- New notification, "The printer needs you": a print that paused (with the printer's own reason, such as running out of filament), a serious printer error, and lost contact during a print. It's on for your existing notifications; turn it off per notification under Settings.
- A warning when a spool holds too little for a print: on the print page, on the queue card, and in notifications. Another tray with the same filament counts as a backup, so you're only warned when the printer would really run out. It needs a remaining amount from Spoolman or the AMS; the print page says when neither knows.
- New notification five minutes before a print is done. You can turn it off per notification.

## 0.4.0

- AirSlice is now an MCP server (`/api/mcp`) for AI assistants such as Claude Code. Sign in with an API key. The assistant can look up models, slice, queue, start prints once you confirm the plate is clear, read printer status and history, and get a camera picture.
- The assistant can print a file in the colours you name ("red", "white PETG"), matched to the closest loaded spool, and list what's loaded. A colour nothing loaded matches is refused instead of guessed.

## 0.3.6

- The 3D preview keeps ribs, sharp edges and thin walls. Detailed models looked soft, and thin shells (like an egg) showed pits, since the preview merged nearby points. Small, detailed parts no longer take the detail from big ones next to them.
- Opening a detailed plate's preview the first time takes a few seconds longer. After that it comes from the cache.

## 0.3.4

- Fixed the plate preview showing the earlier result after slicing again.
- Print several plates of a file together on one build plate, with their colours.
- A 3MF can now have copies alongside other files on the plate.
- The filament list shows only what the chosen plates use, when the file tells. Painted models and colour changes by layer still list every filament.
- A link with several files or print profiles asks which one to add, and added files show plate pictures.
- Plate and file names wrap instead of being cut off.

## 0.3.3

- Upload several files at once, on the start page or when adding to a plate.
- Each file on a plate has its own card with what applies to it alone, and everything else is under "Whole print", both before and after slicing.
- Filaments printing from the same tray are now merged when slicing, so a print no longer purges and changes filament between them. Time and filament use drop to match. This changes what the printer does, so try a small print first.
- After slicing, choosing trays that give another number of colours slices again by itself. Choosing another tray with the same number of colours updates the cost straight away, using the spool's price from Spoolman.

## 0.3.2

- Fixed "couldn't read the added model" for 3MF files from Bambu Studio with settings OrcaSlicer refuses.
- Fixed "the models don't all fit" when it was really the prime tower running off the bed. AirSlice now slices once more without a tower and says so in the job log. Filament changes still purge, but colours may come out a bit less clean.

## 0.3.1

- "More models on this plate" is now a button that opens a dialog: paste a link or choose a file, pick from your MakerWorld collections, or pick an earlier model from History.

## 0.3.0

- Add more models to a plate: on the slicing page, paste a link or choose a file, and set copies and a tray for each. OrcaSlicer arranges the plate and says so if everything doesn't fit.
- The first model keeps its colours and settings. An added model prints in one colour, so a multi-colour one loses its colours, and a 3MF project as the first model can't have copies while models are added.

## 0.2.1

- MakerWorld downloads work again. MakerWorld stopped sending a download link through the route AirSlice used, which showed up as "couldn't reach the file host".
- A missing or unusable download link now says so, instead of blaming the network.

## 0.2.0

- History on a phone: swipe a job to the left to remove it, instead of to the right.
- Prints started on the printer or from another app are followed too and kept in History, marked "Started outside AirSlice", with a photo of the result, notifications and the plug's auto-off. AirSlice doesn't know their filament, so their cost is print time only.
- Prints that ran now stay in History for good, until you remove them. The "Jobs to keep" option only counts jobs that never printed.
- Multi-colour prints show how much filament went to purging and the prime tower.
- New API for the print history (`GET /api/prints`), and `POST /api/prints/import` with `tools/import-arkiv` for prints from an older log.
- New Home Assistant integration in `custom_components/airslice`: print sensors, `airslice.list_prints` and result photos as media.

## 0.1.3

- Changing an AMS HT's filament or re-reading its RFID tag now goes to the right unit.
- A job that's printing can't be removed from the history, by swipe or ✕, so its result isn't lost. A second finger on a row no longer counts as a swipe.
- Printing from an AMS that's drying is refused straight away instead of after the upload, and automatic tray matching picks another unit when one has the same filament.
- The Dry button no longer stays hidden when an AMS shows up late after the printer starts, an unrelated printer message no longer counts as a failed drying start, and the dialog doesn't reopen by itself after a print or drying elsewhere.
- PCTG is suggested 65 °C for drying instead of 80 °C.

## 0.1.2

- History on a phone: swipe a job to the right to remove it. On a computer the ✕ button stays and now asks first.
- Unlinking the Bambu Lab account or removing the Thingiverse token asks first.
- Drying an AMS 2 Pro or AMS HT shows on the printer card with the time left, also when it was started on the printer's screen. A print can't start from a unit that's drying, and the plug's auto-off waits for drying to finish.
- Start and stop drying from the printer card, with temperature, hours and spool turning suggested from what's loaded. Not on a P1S or P1P, which ignore the command (start it on the printer's screen). Unconfirmed on other printers, so AirSlice says so first and asks you to report whether it worked.
- The plug's auto-off no longer turns off a plug you turned on by hand, after something else had turned it off following a print.

## 0.1.1

- A job from MakerWorld, Printables or Thingiverse links to its other files ("Other files", "Other print profiles") and to the original page. On the model page, files you printed before are marked, and a file you haven't printed is picked first.
- The history shows the model's own picture instead of the camera photo of the result (the photo is still on the job's page).
- "Printing on…" notifications carry the plate render, or the site's picture of the file or model, instead of a camera picture of the empty bed. "Sliced" and "printer is free" notifications fall back to the site's picture too.
- Fixed: Printables and Thingiverse pictures didn't show once the app's service worker was active.

## 0.1.0

First alpha.

- Print from MakerWorld, Printables and Thingiverse links, MakerWorld collections (Explore), or an uploaded 3MF, STL, OBJ or STEP file. Model pages with pictures, description and every print profile with time and weight.
- Slices on the server with OrcaSlicer: the file's own settings when it was made for your printer, otherwise your printer's profile; pre-sliced files go straight through. Quick settings and presets, build plates, objects per plate, copies, size and rotation.
- Filament matched to the AMS trays, adjustable before printing; tray editing, RFID re-read and Spoolman.
- 3D plate preview and a layer view of the sliced G-code.
- A queue by printer type: any free printer of the type takes the next print once someone confirms the plate is clear; a queued print can be sliced again for another type.
- Live printer cards: progress, temperatures, fans, AMS, camera, HMS messages in plain words with wiki links, pause, resume, stop, speed and light. Printer setup under Settings → Printers, with a connection test that also catches Developer Mode being off.
- Notifications through Home Assistant, Telegram, ntfy or the browser (Web Push), with pictures and times, per target events and language.
- History with costs, energy, result photos, favourites, print again and statistics.
- Optional Home Assistant power plug: turned on for a print, off after a cool-down.
- Settings → About → Copy diagnostics for bug reports, with secrets left out.
- Runs as a Home Assistant add-on or a Docker container; English and Swedish.
