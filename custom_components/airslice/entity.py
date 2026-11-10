"""What every entity of a printer shares, and adding them as printers appear."""

from __future__ import annotations

from collections.abc import Callable, Iterable
from typing import Any

from homeassistant.core import callback
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity import Entity
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity
from homeassistant.util import slugify

from .const import DOMAIN
from .coordinator import AirSliceConfigEntry, LiveCoordinator


def status_of(printer: dict[str, Any]) -> dict[str, Any]:
    return printer.get("status") or {}


def is_busy(printer: dict[str, Any]) -> bool:
    return status_of(printer).get("state") in ("RUNNING", "PREPARE", "PAUSE", "SLICING")


def trays_of(printer: dict[str, Any]) -> list[dict[str, Any]]:
    status = status_of(printer)
    return [*(status.get("trays") or []), *([status["external"]] if status.get("external") else [])]


def tray_label(tray: dict[str, Any]) -> str:
    """AMS A1, AMS HT 1 or External, as the printer's own screen has them."""
    ams, slot = tray.get("ams"), tray.get("slot")
    if ams is None or slot is None or tray["id"] == 254:
        return "External"
    if ams >= 128:
        return f"AMS HT {ams - 127}"
    return f"AMS {chr(ord('A') + ams)}{slot + 1}"


def device_info(entry_id: str, printer: dict[str, Any]) -> DeviceInfo:
    return DeviceInfo(
        identifiers={(DOMAIN, f"{entry_id}_{printer['id']}")},
        name=printer["name"],
        manufacturer="AirSlice",
        model=printer.get("printer_model") or None,
    )


class PrinterEntity(CoordinatorEntity[LiveCoordinator]):
    """One thing about one printer. Unavailable while the printer isn't
    connected, unless `_always` says it still means something then."""

    _attr_has_entity_name = True
    _always = False

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], key: str, domain: str) -> None:
        super().__init__(coordinator)
        self.printer_id = printer["id"]
        entry_id = coordinator.config_entry.entry_id
        self._attr_unique_id = f"{entry_id}_{printer['id']}_{key}"
        # English entity ids whatever the system language.
        self.entity_id = f"{domain}.airslice_{slugify(printer['name'])}_{key}"
        self._attr_device_info = device_info(entry_id, printer)

    @property
    def printer(self) -> dict[str, Any]:
        return self.coordinator.data.printers[self.printer_id]

    @property
    def status(self) -> dict[str, Any]:
        return status_of(self.printer)

    @property
    def available(self) -> bool:
        if not super().available or self.printer_id not in self.coordinator.data.printers:
            return False
        return self._always or bool(self.status.get("connected"))


@callback
def add_entities_as_found(
    entry: AirSliceConfigEntry,
    add: AddConfigEntryEntitiesCallback,
    build: Callable[[LiveCoordinator, dict[str, Any]], Iterable[Entity]],
) -> None:
    """Add a printer's entities now, and new ones (a new printer or AMS) later."""
    live = entry.runtime_data.live
    known: set[str] = set()

    @callback
    def add_new() -> None:
        new = [
            e
            for printer in live.data.printers.values()
            for e in build(live, printer)
            if e.unique_id not in known
        ]
        known.update(e.unique_id for e in new)
        if new:
            add(new)

    add_new()
    entry.async_on_unload(live.async_add_listener(add_new))
