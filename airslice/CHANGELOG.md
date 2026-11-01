# Changelog

## Unreleased

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
