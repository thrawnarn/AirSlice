"""What just happened on a printer, for automations."""

from __future__ import annotations

from typing import Any

from homeassistant.components.event import EventEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .const import EVENT_TYPES
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
        lambda live, printer: [PrinterEvent(live, printer, "event", "event")],
    )


class PrinterEvent(PrinterEntity, EventEntity):
    _always = True
    _attr_translation_key = "printer_event"
    _attr_event_types = EVENT_TYPES

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        self.async_on_remove(self.coordinator.async_listen_events(self.printer_id, self._on_event))

    @callback
    def _on_event(self, event: dict[str, Any]) -> None:
        if event["kind"] not in EVENT_TYPES:
            return
        self._trigger_event(event["kind"], {"title": event.get("title"), "message": event.get("message"), "job_id": event.get("job_id")})
        self.async_write_ha_state()
