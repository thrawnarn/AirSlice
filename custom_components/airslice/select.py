"""The print speed."""

from __future__ import annotations

from homeassistant.components.select import SelectEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .api import AirSliceError
from .coordinator import AirSliceConfigEntry
from .entity import PrinterEntity, add_entities_as_found

SPEEDS = ["silent", "standard", "sport", "ludicrous"]


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    add_entities_as_found(
        entry,
        async_add_entities,
        lambda live, printer: [SpeedSelect(live, printer, "printing_speed", "select")],
    )


class SpeedSelect(PrinterEntity, SelectEntity):
    _attr_translation_key = "printing_speed"
    _attr_icon = "mdi:speedometer"
    _attr_options = SPEEDS

    @property
    def current_option(self) -> str | None:
        level = self.status.get("speed_level")
        return SPEEDS[level - 1] if level else None

    async def async_select_option(self, option: str) -> None:
        try:
            await self.coordinator.api.control(self.printer_id, f"speed_{SPEEDS.index(option) + 1}")
        except AirSliceError as err:
            raise HomeAssistantError(str(err)) from err
        await self.coordinator.async_request_refresh()
