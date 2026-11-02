"""Reads the print history from AirSlice."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
import logging
from typing import Any

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import ConfigEntryAuthFailed
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .api import AirSliceApi, AirSliceAuthError, AirSliceError, AirSliceTooOld
from .const import DOMAIN, SCAN_INTERVAL_SECONDS
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
