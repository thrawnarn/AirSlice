"""Reads the print history from AirSlice."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
import logging
from collections.abc import Callable
from typing import Any

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import ConfigEntryAuthFailed
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .api import AirSliceApi, AirSliceAuthError, AirSliceError, AirSliceTooOld
from .const import DOMAIN, SCAN_INTERVAL_SECONDS
from .const import EVENT_AIRSLICE, LIVE_INTERVAL_SECONDS
from .prints import to_record

_LOGGER = logging.getLogger(__name__)

type AirSliceConfigEntry = ConfigEntry[AirSliceCoordinator]


@dataclass
class AirSliceData:
    printers: dict[str, str]
    prints: list[dict[str, Any]]


class AirSliceCoordinator(DataUpdateCoordinator[AirSliceData]):
    config_entry: AirSliceConfigEntry

    def __init__(self, hass: HomeAssistant, entry: AirSliceConfigEntry, api: AirSliceApi) -> None:
        super().__init__(
            hass,
            _LOGGER,
            config_entry=entry,
            name=DOMAIN,
            update_interval=timedelta(seconds=SCAN_INTERVAL_SECONDS),
        )
        self.api = api
        self.live: LiveCoordinator

    async def _async_update_data(self) -> AirSliceData:
        try:
            printers = {p["id"]: p["name"] for p in await self.api.printers()}
            prints = await self.api.prints()
        except AirSliceAuthError as err:
            raise ConfigEntryAuthFailed("AirSlice refused the API key") from err
        except AirSliceTooOld as err:
            raise UpdateFailed("Update the AirSlice add-on: this version has no print history API") from err
        except AirSliceError as err:
            raise UpdateFailed(str(err)) from err
        entry_id = self.config_entry.entry_id
        return AirSliceData(printers, [to_record(entry_id, p, printers) for p in prints])


@dataclass
class LiveData:
    printers: dict[str, dict[str, Any]]
    queues: dict[str, list[dict[str, Any]]]
    # printer id -> tray id (as text) -> Spoolman spool
    spools: dict[str, dict[str, dict[str, Any]]]


class LiveCoordinator(DataUpdateCoordinator[LiveData]):
    """What the printers are doing now, and the events since the last look."""

    config_entry: AirSliceConfigEntry

    def __init__(self, hass: HomeAssistant, entry: AirSliceConfigEntry, api: AirSliceApi) -> None:
        super().__init__(
            hass,
            _LOGGER,
            config_entry=entry,
            name=f"{DOMAIN}_live",
            update_interval=timedelta(seconds=LIVE_INTERVAL_SECONDS),
        )
        self.api = api
        self._after: int | None = None
        self._event_listeners: dict[str, list[Callable[[dict[str, Any]], None]]] = {}

    @callback
    def async_listen_events(self, printer_id: str, listener: Callable[[dict[str, Any]], None]) -> Callable[[], None]:
        listeners = self._event_listeners.setdefault(printer_id, [])
        listeners.append(listener)
        return lambda: listeners.remove(listener)

    async def _extras(self, printer_id: str) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
        """The queue and spools are extras: without them the printer still counts."""
        try:
            queue = await self.api.queue(printer_id)
        except AirSliceAuthError:
            raise
        except AirSliceError:
            queue = []
        try:
            spools = (await self.api.spools(printer_id)).get("trays") or {}
        except AirSliceAuthError:
            raise
        except AirSliceError:
            spools = {}
        return queue, spools

    async def _events(self) -> None:
        try:
            res = await self.api.events(self._after)
        except AirSliceAuthError:
            raise
        except AirSliceError:
            return  # older AirSlice, or a hiccup: the next look catches up
        self._after = res.get("latest", self._after)
        for event in res.get("events", []):
            printer = (self.data.printers.get(event["printer_id"], {}) if self.data else {}).get("name")
            self.hass.bus.async_fire(EVENT_AIRSLICE, {**event, "printer": printer})
            for listener in list(self._event_listeners.get(event["printer_id"], [])):
                listener(event)

    async def _async_update_data(self) -> LiveData:
        try:
            printers = {p["id"]: p for p in await self.api.printers()}
            queues, spools = {}, {}
            for printer_id in printers:
                queues[printer_id], spools[printer_id] = await self._extras(printer_id)
            await self._events()
        except AirSliceAuthError as err:
            raise ConfigEntryAuthFailed("AirSlice refused the API key") from err
        except AirSliceError as err:
            raise UpdateFailed(str(err)) from err
        return LiveData(printers, queues, spools)
