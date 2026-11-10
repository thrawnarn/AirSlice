"""Pause, resume, stop, refresh, and starting the next print in the queue."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

from homeassistant.components.button import ButtonEntity, ButtonEntityDescription
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .api import AirSliceError
from .coordinator import AirSliceConfigEntry, LiveCoordinator
from .entity import PrinterEntity, add_entities_as_found, is_busy, status_of


def state_is(*states: str) -> Callable[[dict[str, Any], list[dict[str, Any]]], bool]:
    return lambda printer, _queue: status_of(printer).get("state") in states


def control(action: str) -> Callable[[LiveCoordinator, str], Awaitable[None]]:
    return lambda live, printer_id: live.api.control(printer_id, action)


@dataclass(frozen=True, kw_only=True)
class AirSliceButtonDescription(ButtonEntityDescription):
    press_fn: Callable[[LiveCoordinator, str], Awaitable[None]]
    can_press: Callable[[dict[str, Any], list[dict[str, Any]]], bool] = lambda _printer, _queue: True


BUTTONS: tuple[AirSliceButtonDescription, ...] = (
    AirSliceButtonDescription(key="pause_printing", translation_key="pause_printing", icon="mdi:pause", press_fn=control("pause"), can_press=state_is("RUNNING", "PREPARE")),
    AirSliceButtonDescription(key="resume_printing", translation_key="resume_printing", icon="mdi:play", press_fn=control("resume"), can_press=state_is("PAUSE")),
    AirSliceButtonDescription(key="stop_printing", translation_key="stop_printing", icon="mdi:stop", press_fn=control("stop"), can_press=lambda p, _q: is_busy(p)),
    AirSliceButtonDescription(key="refresh", translation_key="refresh", icon="mdi:refresh", entity_category=EntityCategory.DIAGNOSTIC, press_fn=control("refresh")),
    # The queue never starts by itself: whoever presses this has cleared the plate.
    AirSliceButtonDescription(
        key="start_next",
        translation_key="start_next",
        icon="mdi:skip-next",
        press_fn=lambda live, printer_id: live.api.start_next(printer_id),
        can_press=lambda p, q: bool(q) and not is_busy(p),
    ),
)


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    add_entities_as_found(
        entry,
        async_add_entities,
        lambda live, printer: [AirSliceButton(live, printer, d) for d in BUTTONS],
    )


class AirSliceButton(PrinterEntity, ButtonEntity):
    entity_description: AirSliceButtonDescription

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], description: AirSliceButtonDescription) -> None:
        super().__init__(coordinator, printer, description.key, "button")
        self.entity_description = description

    @property
    def available(self) -> bool:
        queue = self.coordinator.data.queues.get(self.printer_id, []) if self.coordinator.data else []
        return super().available and self.entity_description.can_press(self.printer, queue)

    async def async_press(self) -> None:
        try:
            await self.entity_description.press_fn(self.coordinator, self.printer_id)
        except AirSliceError as err:
            raise HomeAssistantError(str(err)) from err
        await self.coordinator.async_request_refresh()
