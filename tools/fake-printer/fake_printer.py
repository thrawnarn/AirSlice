"""Simulated Bambu Lab printer for testing AirSlice without hardware.

Serves implicit FTPS on 127.0.0.1:990, MQTT over TLS on 127.0.0.1:8883 and
the P1/A1 camera stream over TLS on 127.0.0.1:6000,
behaving like a P1S in LAN-only mode: reports status and AMS trays, accepts
uploads, and reacts to project_file / pause / resume / stop / ledctrl.
Uploads and every MQTT request are recorded in ./out/.

Add it in AirSlice as: host 127.0.0.1, serial 01P00A123456789,
access code 12345678, profile "Bambu Lab P1S 0.4 nozzle".

Environment:
  FTP_MODE=reuse  (default) data channel must resume the TLS session (vsftpd, X1/P1)
  FTP_MODE=clear  refuse PROT P, like the A1 series
  PRINT_SECONDS=N finish a print after N seconds
  AMS_MODULE=n3f/0 what the AMS is (default n3f/0 = AMS 2 Pro, ams/0 = original AMS)
  AMS_HT=1        add an AMS HT (unit 128)
  DRY_REFUSE=why  refuse ams_filament_drying with this reason
  HMS=attr:code   report an active HMS message (hex, e.g. 05000500:00010007)

Port 990 is privileged: run as root on Linux (e.g. sudo), or in a container.
"""
import asyncio, json, os, threading, logging

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get("FAKE_PRINTER_OUT", os.path.join(HERE, "out"))
os.makedirs(OUT, exist_ok=True)

# Self-signed X.509 v1 certificate with the serial as CN, like the real
# printer's (v1 is what broke rustls/webpki; OpenSSL 3 can only make v3, so
# it is checked in). The backend's tls.rs tests use the same pair.
CERT = os.path.join(HERE, "fixtures", "printer-cert.pem")
KEY = os.path.join(HERE, "fixtures", "printer-key.pem")
SERIAL = "01P00A123456789"
CODE = "12345678"
MODE = os.environ.get("FTP_MODE", "reuse")  # reuse | clear (A1-like: refuse PROT P)
# Run a started print for this many seconds, then report FINISH (0 = run forever).
PRINT_SECONDS = int(os.environ.get("PRINT_SECONDS", "0"))
HMS = [{"attr": int(a, 16), "code": int(c, 16)} for a, c in (h.split(":") for h in os.environ.get("HMS", "").split(",") if h)]
# Drop the connection on the first N logins, like a printer on weak Wi-Fi
# (tests AirSlice's upload retries).
FAIL_LOGINS = [int(os.environ.get("FAIL_UPLOADS", "0"))]

# ------------------------------------------------------------------ FTPS
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import TLS_FTPHandler, TLS_DTPHandler
from pyftpdlib.servers import FTPServer

class DTP(TLS_DTPHandler):
    def handle_ssl_established(self):
        from OpenSSL._util import lib as _lib
        reused = bool(_lib.SSL_session_reused(self.socket._ssl))
        with open(os.path.join(OUT, "ftp_data_tls.log"), "a") as f:
            f.write(f"session_reused={reused}\n")
        if MODE == "reuse" and not reused:
            # vsftpd require_ssl_reuse=YES behaviour
            self.close()
            return
        super().handle_ssl_established()

class Implicit(TLS_FTPHandler):
    dtp_handler = DTP
    def handle(self):
        if FAIL_LOGINS[0] > 0:
            FAIL_LOGINS[0] -= 1
            self.close()
            return
        self.secure_connection(self.ssl_context)
    def handle_ssl_established(self):
        TLS_FTPHandler.handle(self)
    def ftp_PROT(self, line):
        if MODE == "clear" and line.upper() == "P":
            self.respond("536 PROT P not supported here (fake A1)")
            return
        return super().ftp_PROT(line)

def run_ftp():
    auth = DummyAuthorizer()
    auth.add_user("bblp", CODE, OUT, perm="elradfmw")
    Implicit.authorizer = auth
    from OpenSSL import SSL
    ctx = SSL.Context(SSL.TLS_SERVER_METHOD)
    ctx.set_max_proto_version(SSL.TLS1_2_VERSION)   # like the printer's vsftpd
    ctx.use_certificate_file(CERT)
    ctx.use_privatekey_file(KEY)
    ctx.set_session_id(b"fakeprinter")
    Implicit.ssl_context = ctx
    Implicit.passive_ports = range(50000, 50100)
    Implicit.masquerade_address = "10.255.255.1"  # a bogus advertised address, must be ignored
    server = FTPServer(("127.0.0.1", 990), Implicit)
    server.serve_forever()

# ---------------------------------------------------------------- camera
CAMERA_JPEG = os.path.join(HERE, "fixtures", "camera.jpg")

def run_camera():
    """P1/A1 camera: TLS on 6000, an 80-byte login (u32 LE 0x40, 0x3000, 0, 0,
    "bblp" and the access code padded to 32 bytes), then a frame every second:
    16-byte header with the JPEG size little-endian first, then the JPEG."""
    import socket, ssl, struct, time
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(CERT, KEY)
    with open(CAMERA_JPEG, "rb") as f:
        jpeg = f.read()
    srv = socket.create_server(("127.0.0.1", 6000))

    def serve(conn):
        try:
            with ctx.wrap_socket(conn, server_side=True) as tls:
                login = b""
                while len(login) < 80:
                    chunk = tls.recv(80 - len(login))
                    if not chunk:
                        return
                    login += chunk
                user, code = (login[16:48].rstrip(b"\0"), login[48:80].rstrip(b"\0"))
                if struct.unpack("<II", login[:8]) != (0x40, 0x3000) or user != b"bblp" or code != CODE.encode():
                    return
                while True:
                    tls.sendall(struct.pack("<IIII", len(jpeg), 0, 1, 0) + jpeg)
                    time.sleep(1)
        except OSError:
            pass

    while True:
        conn, _ = srv.accept()
        threading.Thread(target=serve, args=(conn,), daemon=True).start()

# ------------------------------------------------------------------ MQTT
from amqtt.broker import Broker
from amqtt.client import MQTTClient

STATE = {
    "gcode_state": "IDLE", "mc_percent": 0, "mc_remaining_time": 0,
    "nozzle_temper": 24.0, "bed_temper": 23.0, "subtask_name": "",
    "nozzle_target_temper": 0, "bed_target_temper": 0, "nozzle_diameter": "0.4",
    "wifi_signal": "-58dBm", "cooling_fan_speed": "0", "big_fan1_speed": "0", "big_fan2_speed": "0",
    "heatbreak_fan_speed": "0", "spd_lvl": 2, "spd_mag": 100, "stg_cur": 255, "nozzle_type": "hardened_steel",
    "sdcard": True, "print_type": "idle",
    "lights_report": [{"node": "chamber_light", "mode": "off"}],
    "ams": {"tray_now": "255", "ams": [{"id": "0", "humidity": "4", "humidity_raw": "21", "temp": "23.8", "tray": [
        {"id": "0", "tray_type": "PLA", "tray_color": "FFFFFFFF", "remain": 90, "tray_sub_brands": "PLA Basic", "tray_info_idx": "GFA00"},
        {"id": "1", "tray_type": "PLA", "tray_color": "000000FF", "remain": 40},
        {"id": "2", "tray_type": "PETG", "tray_color": "FF0000FF", "remain": 70},
        {"id": "3"},
    ]}]},
    "vt_tray": {"id": "254", "tray_type": "", "tray_color": "00000000"},
    "hms": HMS,
}

AMS_MODULE = os.environ.get("AMS_MODULE", "n3f/0")
DRY_REFUSE = os.environ.get("DRY_REFUSE", "")
if os.environ.get("AMS_HT"):
    STATE["ams"]["ams"].append({"id": "128", "humidity": "3", "humidity_raw": "30", "temp": "24.0", "tray": [
        {"id": "0", "tray_type": "PA-CF", "tray_color": "222222FF", "remain": 60, "tray_temp": "80", "tray_time": "12"},
    ]})

async def run_print(report, subtask):
    for pct in range(10, 101, 10):
        await asyncio.sleep(PRINT_SECONDS / 10)
        if STATE["gcode_state"] != "RUNNING" or STATE.get("subtask_name") != subtask:
            return
        STATE["mc_percent"] = pct
        await report({"mc_percent": pct, "mc_remaining_time": max(0, 42 - pct * 42 // 100), "layer_num": pct, "total_layer_num": 100})
    cool = {"nozzle_target_temper": 0, "bed_target_temper": 0, "nozzle_temper": 80.0, "bed_temper": 45.0,
            "cooling_fan_speed": "0", "big_fan1_speed": "0", "big_fan2_speed": "0"}
    STATE.update(gcode_state="FINISH", **cool)
    await report({"gcode_state": "FINISH", "mc_percent": 100, **cool})
    # The nozzle cools down in a while (power-off waits for that).
    await asyncio.sleep(10)
    STATE.update(nozzle_temper=35.0, bed_temper=30.0)
    await report({"nozzle_temper": 35.0, "bed_temper": 30.0})

async def device():
    c = MQTTClient(client_id="fake-device")
    await c.connect("mqtt://127.0.0.1:1884/")
    await c.subscribe([(f"device/{SERIAL}/request", 0)])
    async def report(patch):
        await c.publish(f"device/{SERIAL}/report", json.dumps({"print": patch}).encode())
    while True:
        msg = await c.deliver_message()
        payload = json.loads(msg.publish_packet.payload.data)
        with open(os.path.join(OUT, "requests.jsonl"), "a") as f:
            f.write(json.dumps(payload) + "\n")
        if payload.get("pushing", {}).get("command") == "pushall":
            await report(STATE)
        if payload.get("info", {}).get("command") == "get_version":
            modules = [{"name": "ota", "sw_ver": "01.10.00.00", "sn": SERIAL}, {"name": AMS_MODULE, "sw_ver": "03.00.21.29", "sn": "AMS"}]
            if os.environ.get("AMS_HT"):
                modules.append({"name": "n3s/128", "sw_ver": "03.00.21.29", "sn": "HT"})
            await c.publish(f"device/{SERIAL}/report", json.dumps({"info": {"command": "get_version", "module": modules}}).encode())
        cmd = payload.get("print", {}).get("command")
        if cmd == "project_file":
            name = payload["print"]["file"]
            ok = os.path.exists(os.path.join(OUT, name))
            if ok:
                STATE.update(gcode_state="PREPARE", subtask_name=payload["print"]["subtask_name"])
                await report({"gcode_state": "PREPARE", "subtask_name": STATE["subtask_name"]})
                await asyncio.sleep(1)
                hot = {"nozzle_temper": 219.0, "nozzle_target_temper": 220, "bed_temper": 54.5, "bed_target_temper": 55,
                       "cooling_fan_speed": "15", "big_fan1_speed": "10", "big_fan2_speed": "5",
                       "heatbreak_fan_speed": "15", "stg_cur": 0, "print_type": "local"}
                STATE.update(gcode_state="RUNNING", mc_percent=1, **hot)
                await report({"gcode_state": "RUNNING", "mc_percent": 1, "mc_remaining_time": 42, **hot})
                if PRINT_SECONDS:
                    asyncio.create_task(run_print(report, STATE["subtask_name"]))
            else:
                await report({"print_error": 0x0500C010})
        elif cmd == "print_speed":
            lvl = int(payload["print"]["param"])
            STATE.update(spd_lvl=lvl, spd_mag={1: 50, 2: 100, 3: 124, 4: 166}.get(lvl, 100))
            await report({"spd_lvl": STATE["spd_lvl"], "spd_mag": STATE["spd_mag"], "command": "print_speed", "result": "success"})
        elif cmd == "ams_filament_setting":
            p = payload["print"]
            if p["ams_id"] == 255:
                tray = STATE["vt_tray"]
            else:
                tray = STATE["ams"]["ams"][p["ams_id"]]["tray"][p["tray_id"]]
            tray.pop("tray_sub_brands", None)
            tray.update(tray_type=p["tray_type"], tray_color=p["tray_color"], tray_info_idx=p["tray_info_idx"],
                        nozzle_temp_min=str(p["nozzle_temp_min"]), nozzle_temp_max=str(p["nozzle_temp_max"]))
            await report({"ams": STATE["ams"], "vt_tray": STATE["vt_tray"]})
        elif cmd == "ams_filament_drying":
            p = payload["print"]
            if DRY_REFUSE:
                await report({"command": cmd, "result": "fail", "reason": DRY_REFUSE})
            else:
                for unit in STATE["ams"]["ams"]:
                    if int(unit["id"]) == p["ams_id"]:
                        unit["dry_time"] = p["duration"] * 60 if p["mode"] else 0
                await report({"ams": STATE["ams"], "command": cmd, "result": "success"})
        elif cmd == "ams_get_rfid":
            # The AMS takes a few seconds to read the tag, then reports as usual.
            await asyncio.sleep(2)
            await report({"ams": STATE["ams"]})
        elif cmd in ("pause", "resume", "stop"):
            # A stopped print ends as FAILED with the "cancelled" code, like a P1.
            new = {"pause": "PAUSE", "resume": "RUNNING", "stop": "FAILED"}[cmd]
            STATE["gcode_state"] = new
            patch = {"gcode_state": new}
            if cmd == "stop":
                STATE["print_error"] = patch["print_error"] = 0x0300400C
            await report(patch)
        if payload.get("system", {}).get("command") == "ledctrl":
            mode = payload["system"]["led_mode"]
            await report({"lights_report": [{"node": "chamber_light", "mode": mode}]})

async def run_mqtt():
    config = {
        "listeners": {
            "default": {"type": "tcp", "bind": "127.0.0.1:1884"},
            "tls": {"type": "tcp", "bind": "127.0.0.1:8883", "ssl": True,
                    "certfile": CERT, "keyfile": KEY},
        },
        "plugins": {"amqtt.plugins.authentication.AnonymousAuthPlugin": {"allow_anonymous": True}},
    }
    broker = Broker(config)
    await broker.start()
    await asyncio.sleep(0.5)
    await device()

if __name__ == "__main__":
    logging.basicConfig(level=logging.WARNING)
    print(f"Simulated printer: FTPS 127.0.0.1:990 ({MODE}), MQTT 127.0.0.1:8883, camera 127.0.0.1:6000, recording to {OUT}")
    threading.Thread(target=run_ftp, daemon=True).start()
    threading.Thread(target=run_camera, daemon=True).start()
    asyncio.run(run_mqtt())
