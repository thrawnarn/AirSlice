"""The print history as Home Assistant shows it. Pure: no Home Assistant here."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from .const import DOMAIN


def _iso(ts: int | None) -> str | None:
    return datetime.fromtimestamp(ts, UTC).isoformat() if ts else None


def media_id(entry_id: str, job_id: str) -> str:
    """Identifier of a print's photo in the media browser."""
    return f"media-source://{DOMAIN}/{entry_id}/{job_id}"


def to_record(entry_id: str, raw: dict[str, Any], printers: dict[str, str]) -> dict[str, Any]:
    """One print from AirSlice, with times as ISO text and its photo as media."""
    return {
        "id": raw["id"],
        "name": raw["name"],
        "printer": printers.get(raw["printer_id"], raw["printer_id"]),
        "printer_id": raw["printer_id"],
        "external": raw.get("external", False),
        "outcome": raw["outcome"],
        "started": _iso(raw.get("started_at")),
        "ended": _iso(raw.get("ended_at")),
        "progress": raw.get("progress"),
        "layer": raw.get("layer"),
        "total_layers": raw.get("total_layers"),
        "error": raw.get("error"),
        "weight": raw.get("weight_g"),
        "usage": raw.get("usage") or [],
        "cost": raw.get("cost"),
        "waste": raw.get("waste"),
        "energy_kwh": raw.get("energy_kwh"),
        "photo": media_id(entry_id, raw["id"]) if raw.get("photo") else None,
        "cover": raw.get("cover"),
        "model_url": raw.get("model_url"),
    }


def select(
    records: list[dict[str, Any]],
    printer_id: str | None = None,
    search: str | None = None,
    limit: int | None = None,
) -> list[dict[str, Any]]:
    """Filter, newest first as AirSlice sends them. Every word of `search` must match."""
    words = (search or "").lower().split()
    found = [
        r
        for r in records
        if (printer_id is None or r["printer_id"] == printer_id)
        and all(w in r["name"].lower() for w in words)
    ]
    return found[:limit] if limit is not None else found


def summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    """What a list of prints adds up to: prints that ran to the end, and all time spent."""
    finished = [r for r in records if r["outcome"] == "finished"]
    hours = 0.0
    for r in records:
        if r["started"] and r["ended"]:
            span = datetime.fromisoformat(r["ended"]) - datetime.fromisoformat(r["started"])
            hours += max(span.total_seconds(), 0) / 3600
    return {
        "finished": len(finished),
        "hours": round(hours, 1),
        "grams": round(sum(r["weight"] or 0 for r in records), 1),
        "cost": round(sum((r["cost"] or {}).get("total") or 0 for r in records), 2),
    }
