"""The printer's camera, as AirSlice sees it."""

from __future__ import annotations

from typing import Any

from homeassistant.components.camera import Camera
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .api import AirSliceError
from .coordinator import AirSliceConfigEntry, LiveCoordinator
from .entity import PrinterEntity, add_entities_as_found


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    add_entities_as_found(
        entry,
        async_add_entities,
        lambda live, printer: [PrinterCamera(live, printer)] if printer.get("camera") else [],
    )


class PrinterCamera(PrinterEntity, Camera):
    _attr_translation_key = "camera"

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any]) -> None:
        PrinterEntity.__init__(self, coordinator, printer, "camera", "camera")
        Camera.__init__(self)

    async def async_camera_image(self, width: int | None = None, height: int | None = None) -> bytes | None:
        try:
            return await self.coordinator.api.camera(self.printer_id)
        except AirSliceError:
            return None
