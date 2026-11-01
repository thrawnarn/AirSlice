# Changelog

## Unreleased

- Changing an AMS HT's filament or re-reading its RFID tag now goes to the right unit.

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
