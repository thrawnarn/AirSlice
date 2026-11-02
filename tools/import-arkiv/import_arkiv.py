#!/usr/bin/env python3
"""Import prints from a Home Assistant Arkiv list (ha-records) into AirSlice.

    import_arkiv.py <.storage/records.xxx> <AirSlice URL> <API key> [--dry-run]
        [--photos <media dir or URL>] [--photos-token <Home Assistant token>]

A list that automation wrote: {name, result, started, ended, weight, usage,
cost, waste, photo}. Safe to run again: a print already there stays as it is,
and so does one AirSlice followed itself (same printer task, ended within 15
minutes). `--photos` is what media-source://media_source/local/ points at: the
media folder, or http://homeassistant:8123/media/local with a long-lived token.
"""

import argparse
from datetime import datetime
import json
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request

OUTCOME = {"finish": "finished", "failed": "failed"}
# AirSlice adds this to the printer's name for a print.
JOB_SUFFIX = re.compile(r"_[0-9a-f]{6}$")
MEDIA = "media-source://media_source/local/"


def ts(text):
    return int(datetime.fromisoformat(text).timestamp()) if text else None


def as_dict(value):
    """Templates sometimes stored these as JSON text."""
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except ValueError:
            return {}
    return value if isinstance(value, dict) else {}


def to_import(rec):
    d = rec["data"]
    cost = as_dict(d.get("cost"))
    waste = as_dict(d.get("waste"))
    ended = ts(d.get("ended") or rec["created"])
    started = ts(d.get("started")) or ended
    task = d.get("name") or "Print"
    usage = d.get("usage")
    if isinstance(usage, str):
        usage = json.loads(usage or "[]")
    body = {
        "key": f"arkiv:{rec['id']}",
        "name": JOB_SUFFIX.sub("", task),
        "task": task,
        "outcome": OUTCOME.get(d.get("result"), "unknown"),
        "started_at": min(started, ended),
        "ended_at": ended,
        "progress": int(d["progress"]) if d.get("progress") is not None else None,
        "layer": d.get("layer"),
        "total_layers": d.get("total_layers"),
        "weight_g": d.get("weight"),
        "usage": [
            {"slot": u.get("slot") or "", "type": u.get("type") or "", "color": u.get("color") or "", "grams": float(u["grams"])}
            for u in usage or []
            if float(u.get("grams") or 0) > 0
        ],
    }
    if cost.get("total") is not None:
        body["cost"] = {
            "filament": cost.get("filament"),
            "machine": cost.get("machine"),
            "total": cost["total"],
            "currency": "kr",
            "actual": True,
        }
    if cost.get("waste_g"):
        body["waste"] = {
            "changes": int(waste.get("changes") or 0),
            "flush_g": float(waste.get("flush_g") or 0),
            "tower_g": float(waste.get("tower_g") or 0),
            "cost": cost.get("waste"),
        }
    return body


def send(url, key, path, data, content_type="application/json", method="POST"):
    req = urllib.request.Request(
        f"{url}/api/{path}", data=data, method=method, headers={"Authorization": f"Bearer {key}", "Content-Type": content_type}
    )
    with urllib.request.urlopen(req, timeout=30) as res:
        return json.load(res)


def read_photo(base, token, name):
    """The picture's bytes, or None when it's missing or empty."""
    if base.startswith(("http://", "https://")):
        url = f"{base.rstrip('/')}/{urllib.parse.quote(name)}"
        req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"} if token else {})
        try:
            with urllib.request.urlopen(req, timeout=30) as res:
                return res.read() or None
        except urllib.error.URLError as err:
            print(f"  no photo: {name}: {err}")
            return None
    file = Path(base) / name
    return (file.read_bytes() or None) if file.is_file() else None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("store", type=Path)
    ap.add_argument("url")
    ap.add_argument("key")
    ap.add_argument("--photos", help="the media folder, or its URL")
    ap.add_argument("--photos-token", help="Home Assistant token for a photos URL")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    data = json.loads(args.store.read_text())["data"]
    url = args.url.rstrip("/")
    for rec in sorted(data.get("items") or data["records"], key=lambda r: r["created"]):
        body = to_import(rec)
        if args.dry_run:
            print(json.dumps(body, ensure_ascii=False))
            continue
        res = send(url, args.key, "prints/import", json.dumps(body).encode())
        print(res["status"], res["id"], body["name"])
        photo = rec["data"].get("photo") or ""
        if args.photos and photo.startswith(MEDIA):
            picture = read_photo(args.photos, args.photos_token, photo[len(MEDIA):])
            # A print AirSlice followed itself keeps its own picture.
            if picture and res["status"] == "imported":
                send(url, args.key, f"jobs/{res['id']}/photo.jpg", picture, "image/jpeg", "PUT")


if __name__ == "__main__":
    main()
