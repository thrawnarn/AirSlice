"""Sensors of a printer: its print history and what it's doing now."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any

from homeassistant.components.sensor import (
    SensorDeviceClass,
    SensorEntity,
    SensorEntityDescription,
    SensorStateClass,
)
from homeassistant.const import (
    PERCENTAGE,
    SIGNAL_STRENGTH_DECIBELS_MILLIWATT,
    EntityCategory,
    UnitOfLength,
    UnitOfMass,
    UnitOfTemperature,
    UnitOfTime,
)
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity
from homeassistant.util import dt as dt_util, slugify

from .const import DOMAIN
from .coordinator import AirSliceConfigEntry, AirSliceCoordinator, LiveCoordinator
from .entity import PrinterEntity, add_entities_as_found, status_of, tray_label, trays_of
from .prints import select, summary

# From the printer's `stg_cur`, as AirSlice's own sensor page has them.
STAGES = {
    0: "printing",
    1: "levelling_the_bed",
    2: "heating_the_bed",
    3: "vibration_compensation",
    4: "changing_filament",
    5: "paused",
    6: "paused_filament_ran_out",
    7: "heating_the_nozzle",
    8: "calibrating_extrusion",
    9: "scanning_the_bed",
    10: "inspecting_the_first_layer",
    11: "identifying_the_build_plate",
    12: "calibrating_the_micro_lidar",
    13: "homing",
    14: "cleaning_the_nozzle_tip",
    15: "checking_the_nozzle_temperature",
    16: "paused_by_you",
    17: "paused_front_cover_fell_off",
    18: "calibrating_the_micro_lidar",
    19: "calibrating_extrusion_flow",
    20: "paused_nozzle_temperature_fault",
    21: "paused_bed_temperature_fault",
    22: "unloading_filament",
    23: "paused_skipped_step",
    24: "loading_filament",
    25: "calibrating_motor_noise",
    26: "paused_ams_lost",
    27: "paused_heatbreak_fan_too_slow",
    29: "cooling_the_chamber",
    30: "paused_by_the_g_code",
    32: "paused_filament_on_the_nozzle",
    34: "paused_first_layer_error",
    35: "paused_nozzle_clogged",
}


def print_status(printer: dict[str, Any]) -> str:
    status = status_of(printer)
    if not status.get("connected"):
        return "offline"
    return (status.get("state") or "unknown").lower()


def active_tray(printer: dict[str, Any]) -> str | None:
    return next((tray_label(t) for t in trays_of(printer) if t.get("active")), "none")


def job_time(field: str) -> Callable[[dict[str, Any]], datetime | None]:
    def value(printer: dict[str, Any]) -> datetime | None:
        ts = (printer.get("current_job") or {}).get(field)
        return datetime.fromtimestamp(ts, UTC) if ts else None

    return value


def job_value(field: str) -> Callable[[dict[str, Any]], Any]:
    return lambda printer: (printer.get("current_job") or {}).get(field)


def from_status(field: str) -> Callable[[dict[str, Any]], Any]:
    return lambda printer: status_of(printer).get(field)


@dataclass(frozen=True, kw_only=True)
class LiveSensorDescription(SensorEntityDescription):
    value_fn: Callable[[dict[str, Any]], Any]
    always: bool = False


TEMPERATURE = {
    "device_class": SensorDeviceClass.TEMPERATURE,
    "native_unit_of_measurement": UnitOfTemperature.CELSIUS,
    "state_class": SensorStateClass.MEASUREMENT,
}
FAN = {
    "native_unit_of_measurement": PERCENTAGE,
    "state_class": SensorStateClass.MEASUREMENT,
    "entity_category": EntityCategory.DIAGNOSTIC,
    "icon": "mdi:fan",
}

# Keys follow ha-bambulab's where it has the same thing.
SENSORS: tuple[LiveSensorDescription, ...] = (
    LiveSensorDescription(key="print_status", translation_key="print_status", icon="mdi:printer-3d", value_fn=print_status, always=True),
    LiveSensorDescription(key="task_name", translation_key="task_name", icon="mdi:file-outline", value_fn=from_status("task_name")),
    LiveSensorDescription(key="print_progress", translation_key="print_progress", native_unit_of_measurement=PERCENTAGE, state_class=SensorStateClass.MEASUREMENT, icon="mdi:progress-clock", value_fn=from_status("percent")),
    LiveSensorDescription(key="remaining_time", translation_key="remaining_time", device_class=SensorDeviceClass.DURATION, native_unit_of_measurement=UnitOfTime.MINUTES, value_fn=from_status("remaining_minutes")),
    LiveSensorDescription(key="start_time", translation_key="start_time", device_class=SensorDeviceClass.TIMESTAMP, value_fn=job_time("started_at")),
    LiveSensorDescription(key="current_layer", translation_key="current_layer", icon="mdi:layers-triple", state_class=SensorStateClass.MEASUREMENT, value_fn=from_status("layer")),
    LiveSensorDescription(key="total_layer_count", translation_key="total_layer_count", icon="mdi:layers-triple", state_class=SensorStateClass.MEASUREMENT, value_fn=from_status("total_layers")),
    LiveSensorDescription(key="print_weight", translation_key="print_weight", device_class=SensorDeviceClass.WEIGHT, native_unit_of_measurement=UnitOfMass.GRAMS, value_fn=job_value("weight_g")),
    LiveSensorDescription(key="print_length", translation_key="print_length", device_class=SensorDeviceClass.DISTANCE, native_unit_of_measurement=UnitOfLength.METERS, value_fn=job_value("length_m")),
    LiveSensorDescription(key="current_stage", translation_key="current_stage", icon="mdi:progress-wrench", value_fn=lambda p: STAGES.get(status_of(p).get("stage"), status_of(p).get("stage"))),
    LiveSensorDescription(key="active_tray", translation_key="active_tray", icon="mdi:printer-3d-nozzle", value_fn=active_tray),
    LiveSensorDescription(key="nozzle_temperature", translation_key="nozzle_temperature", value_fn=from_status("nozzle_temp"), **TEMPERATURE),
    LiveSensorDescription(key="nozzle_target_temperature", translation_key="nozzle_target_temperature", value_fn=from_status("nozzle_target"), **TEMPERATURE),
    LiveSensorDescription(key="bed_temperature", translation_key="bed_temperature", value_fn=from_status("bed_temp"), **TEMPERATURE),
    LiveSensorDescription(key="bed_target_temperature", translation_key="bed_target_temperature", value_fn=from_status("bed_target"), **TEMPERATURE),
    LiveSensorDescription(key="speed_percent", translation_key="speed_percent", native_unit_of_measurement=PERCENTAGE, icon="mdi:speedometer", value_fn=from_status("speed_percent")),
    LiveSensorDescription(key="cooling_fan_speed", translation_key="cooling_fan_speed", value_fn=from_status("fan_part"), **FAN),
    LiveSensorDescription(key="aux_fan_speed", translation_key="aux_fan_speed", value_fn=from_status("fan_aux"), **FAN),
    LiveSensorDescription(key="chamber_fan_speed", translation_key="chamber_fan_speed", value_fn=from_status("fan_chamber"), **FAN),
    LiveSensorDescription(key="heatbreak_fan_speed", translation_key="heatbreak_fan_speed", value_fn=from_status("fan_heatbreak"), **FAN),
    LiveSensorDescription(key="wifi_signal", translation_key="wifi_signal", device_class=SensorDeviceClass.SIGNAL_STRENGTH, native_unit_of_measurement=SIGNAL_STRENGTH_DECIBELS_MILLIWATT, state_class=SensorStateClass.MEASUREMENT, entity_category=EntityCategory.DIAGNOSTIC, value_fn=from_status("wifi_dbm")),
    LiveSensorDescription(key="nozzle_size", translation_key="nozzle_size", icon="mdi:printer-3d-nozzle", entity_category=EntityCategory.DIAGNOSTIC, value_fn=from_status("nozzle_diameter")),
    LiveSensorDescription(key="nozzle_type", translation_key="nozzle_type", icon="mdi:printer-3d-nozzle", entity_category=EntityCategory.DIAGNOSTIC, value_fn=from_status("nozzle_type")),
    LiveSensorDescription(key="print_type", translation_key="print_type", icon="mdi:cloud-outline", entity_category=EntityCategory.DIAGNOSTIC, value_fn=from_status("print_type")),
    LiveSensorDescription(key="gcode_filename", translation_key="gcode_filename", icon="mdi:file-code-outline", entity_category=EntityCategory.DIAGNOSTIC, value_fn=from_status("gcode_file")),
)


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
            async_add_entities(TotalHoursSensor(coordinator, p) for p in new)

    add_new_printers()
    entry.async_on_unload(coordinator.async_add_listener(add_new_printers))

    def build(live: LiveCoordinator, printer: dict[str, Any]) -> list[SensorEntity]:
        out: list[SensorEntity] = [LiveSensor(live, printer, d) for d in SENSORS]
        out += [EndTimeSensor(live, printer), QueueSensor(live, printer)]
        out += [TraySensor(live, printer, t["id"]) for t in trays_of(printer)]
        for unit in status_of(printer).get("ams_units") or []:
            out += [AmsSensor(live, printer, unit["id"], unit["name"], d) for d in AMS_SENSORS]
        return out

    add_entities_as_found(entry, async_add_entities, build)


class HistorySensor(CoordinatorEntity[AirSliceCoordinator], SensorEntity):
    _attr_has_entity_name = True
    _key: str

    def __init__(self, coordinator: AirSliceCoordinator, printer_id: str) -> None:
        super().__init__(coordinator)
        self.printer_id = printer_id
        name = coordinator.data.printers[printer_id]
        entry_id = coordinator.config_entry.entry_id
        self._attr_unique_id = f"{entry_id}_{printer_id}_{self._key}"
        # English entity ids whatever the system language.
        self.entity_id = f"sensor.airslice_{slugify(name)}_{self._key}"
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


class PrintsSensor(HistorySensor):
    """How many prints the history holds."""

    _key = "prints"
    _attr_translation_key = "prints"
    _attr_icon = "mdi:printer-3d"

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


class TotalHoursSensor(HistorySensor):
    """Hours in the history, for utility meters. Starts from what AirSlice has followed."""

    _key = "total_hours"
    _attr_translation_key = "total_hours"
    _attr_device_class = SensorDeviceClass.DURATION
    _attr_native_unit_of_measurement = UnitOfTime.HOURS
    _attr_state_class = SensorStateClass.TOTAL_INCREASING
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    @property
    def native_value(self) -> float:
        return summary(self._prints)["hours"]


class LiveSensor(PrinterEntity, SensorEntity):
    entity_description: LiveSensorDescription

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], description: LiveSensorDescription) -> None:
        super().__init__(coordinator, printer, description.key, "sensor")
        self.entity_description = description
        self._always = description.always

    @property
    def native_value(self) -> Any:
        return self.entity_description.value_fn(self.printer)


class EndTimeSensor(PrinterEntity, SensorEntity):
    """When it'll be done. Left alone while the estimate stays within two minutes."""

    _attr_translation_key = "end_time"
    _attr_device_class = SensorDeviceClass.TIMESTAMP

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any]) -> None:
        super().__init__(coordinator, printer, "end_time", "sensor")
        self._end: datetime | None = None

    @property
    def native_value(self) -> datetime | None:
        left = self.status.get("remaining_minutes")
        if self.status.get("state") not in ("RUNNING", "PREPARE", "PAUSE") or not left:
            self._end = None
            return None
        end = dt_util.utcnow() + timedelta(minutes=left)
        if self._end is None or abs(end - self._end) > timedelta(minutes=2):
            self._end = end.replace(second=0, microsecond=0)
        return self._end


class QueueSensor(PrinterEntity, SensorEntity):
    _always = True
    _attr_translation_key = "queue"
    _attr_icon = "mdi:format-list-numbered"
    _attr_native_unit_of_measurement = "prints"

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any]) -> None:
        super().__init__(coordinator, printer, "queue", "sensor")

    @property
    def _queue(self) -> list[dict[str, Any]]:
        return self.coordinator.data.queues.get(self.printer_id, [])

    @property
    def native_value(self) -> int:
        return len(self._queue)

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        return {
            "prints": [
                {
                    "title": q.get("title"),
                    "plate": q.get("plate_label"),
                    "minutes": round(q["prediction_secs"] / 60) if q.get("prediction_secs") else None,
                    "weight": q.get("weight_g"),
                    "filament_ok": q.get("filament_ok"),
                }
                for q in self._queue
            ]
        }


class TraySensor(PrinterEntity, SensorEntity):
    """What's loaded in a tray, and how much is left."""

    _attr_translation_key = "tray"
    _attr_icon = "mdi:printer-3d-nozzle"

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], tray_id: int) -> None:
        super().__init__(coordinator, printer, "external_spool" if tray_id == 254 else f"tray_{tray_id}", "sensor")
        self.tray_id = tray_id
        self._attr_translation_placeholders = {"tray": self._label(printer)}

    def _label(self, printer: dict[str, Any]) -> str:
        return next((tray_label(t) for t in trays_of(printer) if t["id"] == self.tray_id), str(self.tray_id))

    @property
    def _tray(self) -> dict[str, Any] | None:
        return next((t for t in trays_of(self.printer) if t["id"] == self.tray_id), None)

    @property
    def available(self) -> bool:
        return super().available and self._tray is not None

    @property
    def native_value(self) -> str | None:
        tray = self._tray
        if tray is None:
            return None
        return tray.get("name") or tray.get("type") or "empty"

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        tray = self._tray or {}
        spool = self.coordinator.data.spools.get(self.printer_id, {}).get(str(self.tray_id)) or {}
        left = spool.get("remaining_g")
        if left is None and tray.get("remain") is not None:
            left = tray["remain"] / 100 * (tray.get("spool_g") or 1000)
        return {
            "tray": self._label(self.printer),
            "type": tray.get("type") or None,
            "color": tray.get("color") or None,
            "percent": tray.get("remain"),
            "remaining_g": round(left, 1) if left is not None else None,
            "spoolman_id": spool.get("id"),
            "active": bool(tray.get("active")),
        }


@dataclass(frozen=True, kw_only=True)
class AmsSensorDescription(SensorEntityDescription):
    field: str


AMS_SENSORS: tuple[AmsSensorDescription, ...] = (
    AmsSensorDescription(key="humidity", translation_key="ams_humidity", field="humidity_pct", device_class=SensorDeviceClass.HUMIDITY, native_unit_of_measurement=PERCENTAGE, state_class=SensorStateClass.MEASUREMENT),
    AmsSensorDescription(key="humidity_level", translation_key="ams_humidity_level", field="humidity_level", icon="mdi:water-percent", state_class=SensorStateClass.MEASUREMENT),
    AmsSensorDescription(key="temperature", translation_key="ams_temperature", field="temp", device_class=SensorDeviceClass.TEMPERATURE, native_unit_of_measurement=UnitOfTemperature.CELSIUS, state_class=SensorStateClass.MEASUREMENT),
    AmsSensorDescription(key="drying_remaining", translation_key="ams_drying_remaining", field="dry_minutes", device_class=SensorDeviceClass.DURATION, native_unit_of_measurement=UnitOfTime.MINUTES),
)


class AmsSensor(PrinterEntity, SensorEntity):
    entity_description: AmsSensorDescription

    def __init__(self, coordinator: LiveCoordinator, printer: dict[str, Any], unit: int, name: str, description: AmsSensorDescription) -> None:
        super().__init__(coordinator, printer, f"ams_{unit}_{description.key}", "sensor")
        self.unit = unit
        self.entity_description = description
        self._attr_translation_placeholders = {"ams": name}

    @property
    def native_value(self) -> Any:
        return next((u.get(self.entity_description.field) for u in self.status.get("ams_units") or [] if u["id"] == self.unit), None)
