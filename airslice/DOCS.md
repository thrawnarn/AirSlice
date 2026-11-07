# AirSlice

Send models from MakerWorld, Printables and Thingiverse to a Bambu Lab printer that has no internet access. AirSlice downloads the model, slices it with OrcaSlicer, matches filament to your AMS and starts the print over your LAN.

This is an **alpha**: it's used daily on a P1S, other models haven't been tried yet. When something goes wrong, send **Settings → About → Copy diagnostics** (access codes, tokens and serial numbers are left out).

## Before you start

On the printer:

1. **Settings → LAN Only Mode: on**, then **Developer Mode: on**.
2. Note the **access code**, the **IP address** and the **serial number** (Settings → Device).
3. Give the printer a fixed IP in your router, and block its internet access in your firewall.

## Using it

Open **AirSlice** in the Home Assistant sidebar. You're signed in through Home Assistant, so there's no separate login.

1. **Settings → Printers → Add printer**: enter the IP, serial number, access code and model. Use **Test connection** to check status and uploads.
2. **Settings → Model sites**: sign in with your Bambu Lab account for MakerWorld (it only allows downloads for signed-in users). Printables needs no account. For Thingiverse, create a free app at thingiverse.com/developers and paste its App Token on the same page.
3. **Print**: paste a model link, pick a profile or file, pick a plate, check the filament mapping, start.

## Queue, favourites and print again

- **Add to the queue** puts a sliced print in line for the printer model it was sliced for, so any free printer of that type can take it. The **Print** page shows the queue by printer type: reorder or remove prints, and tap **Start on …** once the plate is clear. A printer card shows the next print with a **Start** button. Nothing in the queue starts on its own.
- A queued print can be sliced again for a free printer of another type (**Slice for …**), with the same plate, objects and settings.
- Star a job to keep it under **History → Favourites**. Favourites are never cleaned up.
- **Print again** copies a finished job, already sliced, so it goes straight to the filament check.
- For STL, OBJ and STEP files you can set the size, number of copies and rotation before slicing, and pick the AMS tray to print with. AirSlice slices with that filament's profile (the exact one for Bambu spools).
- For models with several plates, **Slice and queue all plates** prepares every plate and queues them in order.
- Before slicing, **Quick settings** change layer height, walls, infill, supports and brim for this print. Tap a preset (Draft, Strong, Fine) or adjust and **Save as preset…**. Home Assistant and scripts can pass `"preset": "Strong"` to quick print.
- After slicing, **Show layers** steps through the sliced G-code layer by layer, coloured by filament or by line type (walls, infill, supports…). Tap the picture to look closer.
- The print page shows a 3D preview of the plate: drag to turn it, pinch to zoom. After slicing it shows exactly what will print, in the colours of the trays you picked.

## Camera

Under **Settings → Printers → Edit → Camera**, pick where pictures come from:

- **The printer's camera**: AirSlice reads it directly (P1 and A1 series; no Home Assistant needed). The printer allows one viewer at a time, so if Home Assistant's Bambu Lab integration already shows the camera, use the next option instead.
- **A Home Assistant camera**: any `camera.` entity, such as the Bambu Lab integration's. In the add-on this works as is; in Docker, enter the Home Assistant address and a long-lived access token under **Settings → Notifications**.

The camera shows on the printer card and the job page while printing (tap **Camera** to see it at other times). When a print ends, AirSlice keeps a picture of the result with the job; it shows on the job and in the history.

## Notifications

Under **Settings → Notifications**, add one notification for each person or place; each has its own name, events and camera-picture choice, and its own way of sending:

- **Home Assistant**: a notify service, such as the companion app on your phone. In the add-on nothing else is needed; in Docker, enter the Home Assistant address and a long-lived access token.
- **Telegram**: create a bot with @BotFather, send it a message, and enter its token and your chat id (@userinfobot tells you yours).
- **ntfy**: a topic on ntfy.sh or your own server, followed in the ntfy app. On ntfy.sh anyone who knows the topic name can read it, so use a long, hard-to-guess name or a protected topic with an access token.
- **This browser**: open AirSlice at its own address (not inside Home Assistant) and tap **Notify this device**. The browser shows the notifications itself and a tap opens the print. On iPhone and iPad, add AirSlice to the Home Screen first (Share → Add to Home Screen) and turn notifications on from there; iOS shows text only, no pictures. Each device is added on its own.

Pick which events each one gets. **The first layer is done** comes as soon as layer 2 starts, with a camera picture when the printer has a camera, so a print that doesn't stick can be stopped early. Telegram and ntfy attach the picture themselves; Home Assistant can only show pictures from a Home Assistant camera.

## Phone app and shared links

The Home Assistant panel works in the Companion app. For a standalone home-screen app and for sharing links from the MakerWorld app:

1. In this add-on's **Configuration → Network**, set a host port for `8080/tcp` (for example `8080`).
2. In AirSlice, **Settings → Access → Set a password for direct access** (or set **Password for direct access** in the add-on options).
3. On your phone (on your network or VPN), open `http://<home-assistant-ip>:8080/` and add it to the home screen. Put it behind your reverse proxy with HTTPS for the best experience on iOS.
4. Follow **Settings → On your phone** in AirSlice to create the iOS Shortcut for the share sheet.

Requests through Home Assistant's panel never need the password; the direct port always does (or an API key).

## Options

| Option | |
|---|---|
| Log level | How much detail goes to the add-on log. `debug` shows printer traffic. |
| Password for direct access | Optional fixed password for the direct port. Leave empty to set it in AirSlice. |
| Slicing time limit | Stop slicing after this many seconds. |
| Unprinted jobs to keep | Jobs that never printed kept in History, with their files. Prints that ran are always kept, until you remove them. |

Downloaded and sliced files are excluded from Home Assistant backups. Printers, keys and accounts are included.

## Home Assistant automations

Create an API key under **Settings → API keys**. From Home Assistant itself you can reach the add-on on the internal network without opening the port. The host name is shown on the add-on's Info page (for example `http://local-airslice:8080` for a local add-on, or `http://<repository-id>-airslice:8080`).

```yaml
rest_command:
  airslice_print:
    url: "http://local-airslice:8080/api/print"
    method: post
    headers:
      X-API-Key: !secret airslice_key
    content_type: "application/json"
    payload: '{"url": "{{ url }}"}'

  airslice_start_next:
    url: "http://local-airslice:8080/api/printers/PRINTER_ID/start-next"
    method: post
    headers:
      X-API-Key: !secret airslice_key

sensor:
  - platform: rest
    name: AirSlice printer
    resource: "http://local-airslice:8080/api/printers"
    headers:
      X-API-Key: !secret airslice_key
    value_template: "{{ value_json[0].status.state }}"
    json_attributes_path: "$[0].status"
    json_attributes: [percent, remaining_minutes, task_name, nozzle_temp, bed_temp]
    scan_interval: 30

# A button card (or an NFC tag next to the printer) can call
# rest_command.airslice_start_next once the plate is cleared. The printer
# list's `queue_length` tells you whether anything is waiting.
```

The full API is described in the project README.

## AI assistants (MCP)

AirSlice is also an MCP server at `/api/mcp` on the direct port (or your reverse proxy), for assistants such as Claude Code. Create an API key for it and send it as `Authorization: Bearer as_…`. The assistant can look up models, slice, queue and read printer status. It can start a print only when it says you confirmed the plate is clear. See the README for the setup command and the list of tools.

## Troubleshooting

- **The connection test says the printer refused a command, or a print doesn't start (HMS 0500_0500_0001_0007)**: turn on **Developer Mode** on the printer (Settings → Network, after LAN Only Mode).

- **Printer offline, "access code rejected"**: the code changes when LAN mode is toggled; copy it again from the printer.
- **Upload fails on an A1 / A1 mini**: set **Upload mode** to *Unencrypted data* in the printer's Advanced settings.
- **MakerWorld asks for a CAPTCHA**: Bambu sometimes challenges a server after many requests. It clears within a few hours; meanwhile download the 3MF on your phone and use **Upload 3MF or STL**.
- **Anything else**: **Settings → About → Copy diagnostics** and send the text along with what you did.
