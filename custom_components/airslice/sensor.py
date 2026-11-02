"""One sensor per printer: how many prints its history holds."""

from __future__ import annotations

from typing import Any

from homeassistant.components.sensor import SensorEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity
from homeassistant.util import slugify

from .const import DOMAIN
from .coordinator import AirSliceConfigEntry, AirSliceCoordinator
from .prints import select, summary


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    coordinator = entry.runtime_data
    known: set[str] = set()

    @callback
    def add_new_printers() -> None:
        new = [p for p in coordinator.data.printers if p not in known]
        known.update(new)
        if new:
            async_add_entities(PrintsSensor(coordinator, p) for p in new)

    add_new_printers()
    entry.async_on_unload(coordinator.async_add_listener(add_new_printers))


class PrintsSensor(CoordinatorEntity[AirSliceCoordinator], SensorEntity):
    _attr_has_entity_name = True
    _attr_translation_key = "prints"
    _attr_icon = "mdi:printer-3d"

    def __init__(self, coordinator: AirSliceCoordinator, printer_id: str) -> None:
        super().__init__(coordinator)
        self.printer_id = printer_id
        name = coordinator.data.printers[printer_id]
        entry_id = coordinator.config_entry.entry_id
        self._attr_unique_id = f"{entry_id}_{printer_id}_prints"
        # English entity ids whatever the system language.
        self.entity_id = f"sensor.airslice_{slugify(name)}_prints"
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, f"{entry_id}_{printer_id}")},
            name=name,
            manufacturer="AirSlice",
        )

    @property
    def available(self) -> bool:
        return super().available and self.printer_id in self.coordinator.data.printers

    @property
    def _prints(self) -> list[dict[str, Any]]:
        return select(self.coordinator.data.prints, self.printer_id)

    @property
    def native_value(self) -> int:
        return len(self._prints)

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        prints = self._prints
        latest = prints[0] if prints else None
        return {
            **summary(prints),
            "latest_id": latest and latest["id"],
            "latest_name": latest and latest["name"],
            "latest_outcome": latest and latest["outcome"],
            "latest_ended": latest and latest["ended"],
        }
