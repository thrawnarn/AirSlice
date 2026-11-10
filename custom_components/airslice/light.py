"""The chamber light."""

from __future__ import annotations

from typing import Any

from homeassistant.components.light import ColorMode, LightEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .api import AirSliceError
from .coordinator import AirSliceConfigEntry
from .entity import PrinterEntity, add_entities_as_found


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    add_entities_as_found(
        entry,
        async_add_entities,
        lambda live, printer: [ChamberLight(live, printer, "chamber_light", "light")],
    )


class ChamberLight(PrinterEntity, LightEntity):
    _attr_translation_key = "chamber_light"
    _attr_color_mode = ColorMode.ONOFF
    _attr_supported_color_modes = {ColorMode.ONOFF}

    @property
    def is_on(self) -> bool | None:
        return self.status.get("chamber_light")

    async def _set(self, action: str) -> None:
        try:
            await self.coordinator.api.control(self.printer_id, action)
        except AirSliceError as err:
            raise HomeAssistantError(str(err)) from err
        await self.coordinator.async_request_refresh()

    async def async_turn_on(self, **kwargs: Any) -> None:
        await self._set("light_on")

    async def async_turn_off(self, **kwargs: Any) -> None:
        await self._set("light_off")
