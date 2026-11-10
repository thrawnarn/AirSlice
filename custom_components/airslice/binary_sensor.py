"""Is the printer there, does it have a problem, is the AMS drying."""

from __future__ import annotations

from typing import Any

from homeassistant.components.binary_sensor import BinarySensorDeviceClass, BinarySensorEntity
from homeassistant.const import EntityCategory
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback

from .coordinator import AirSliceConfigEntry, LiveCoordinator
from .entity import PrinterEntity, add_entities_as_found, status_of


async def async_setup_entry(
    hass: HomeAssistant,
    entry: AirSliceConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    def build(live: LiveCoordinator, printer: dict[str, Any]) -> list[BinarySensorEntity]:
        out: list[BinarySensorEntity] = [
            OnlineSensor(live, printer, "online", "binary_sensor"),
            HmsSensor(live, printer, "hms_errors", "binary_sensor"),
            PrintErrorSensor(live, printer, "print_error", "binary_sensor"),
            DryingSensor(live, printer, "ams_drying", "binary_sensor"),
            BackupSensor(live, printer, "ams_filament_backup", "binary_sensor"),
        ]
        out += [UnitDryingSensor(live, printer, u["id"], u["name"]) for u in status_of(printer).get("ams_units") or []]
        return out

    add_entities_as_found(entry, async_add_entities, build)


class OnlineSensor(PrinterEntity, BinarySensorEntity):
    _always = True
    _attr_translation_key = "online"
    _attr_device_class = BinarySensorDeviceClass.CONNECTIVITY
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    @property
    def is_on(self) -> bool:
        return bool(self.status.get("connected"))


class HmsSensor(PrinterEntity, BinarySensorEntity):
    _attr_translation_key = "hms_errors"
    _attr_device_class = BinarySensorDeviceClass.PROBLEM

    @property
    def is_on(self) -> bool:
        return bool(self.status.get("problems"))

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        lang = "sv" if self.hass.config.language.startswith("sv") else "en"
        problems = self.status.get("problems") or []
        out: dict[str, Any] = {"Count": len(problems)}
        for i, p in enumerate(problems, 1):
            texts = p.get("texts") or {}
            out[f"{i}-Code"] = f"HMS_{p['code']}"
            out[f"{i}-Error"] = texts.get(lang) or texts.get("en") or p.get("text")
            out[f"{i}-Wiki"] = p.get("url")
            out[f"{i}-Severity"] = p.get("severity")
        return out


class PrintErrorSensor(PrinterEntity, BinarySensorEntity):
    _attr_translation_key = "print_error"
    _attr_device_class = BinarySensorDeviceClass.PROBLEM

    @property
    def is_on(self) -> bool:
        return bool(self.status.get("print_error"))

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        code = self.status.get("print_error")
        return {"code": f"{code:08X}" if code else None}


class DryingSensor(PrinterEntity, BinarySensorEntity):
    """Any AMS unit drying."""

    _attr_translation_key = "ams_drying"
    _attr_device_class = BinarySensorDeviceClass.RUNNING

    @property
    def is_on(self) -> bool:
        return any(u.get("dry_minutes") for u in self.status.get("ams_units") or [])


class BackupSensor(PrinterEntity, BinarySensorEntity):
    _attr_translation_key = "ams_filament_backup"
    _attr_icon = "mdi:swap-horizontal"
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    @property
    def is_on(self) -> bool | None:
        return self.status.get("ams_backup")


class UnitDryingSensor(PrinterEntity, BinarySensorEntity):
    _attr_translation_key = "ams_unit_drying"
    _attr_device_class = BinarySensorDeviceClass.RUNNING

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], unit: int, name: str) -> None:
        super().__init__(coordinator, printer, f"ams_{unit}_drying", "binary_sensor")
        self.unit = unit
        self._attr_translation_placeholders = {"ams": name}

    @property
    def is_on(self) -> bool:
        return any(u.get("dry_minutes") for u in self.status.get("ams_units") or [] if u["id"] == self.unit)
