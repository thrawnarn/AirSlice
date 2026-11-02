"""`airslice.list_prints`: the prints of a printer, for cards and automations."""

from __future__ import annotations

import voluptuous as vol

from homeassistant.const import ATTR_ENTITY_ID
from homeassistant.core import (
    HomeAssistant,
    ServiceCall,
    ServiceResponse,
    SupportsResponse,
    callback,
)
from homeassistant.exceptions import ServiceValidationError
from homeassistant.helpers import config_validation as cv, entity_registry as er

from .const import ATTR_LIMIT, ATTR_SEARCH, DOMAIN
from .coordinator import AirSliceCoordinator
from .prints import select

LIST_SCHEMA = vol.Schema(
    {
        vol.Required(ATTR_ENTITY_ID): cv.entity_id,
        vol.Optional(ATTR_SEARCH): cv.string,
        vol.Optional(ATTR_LIMIT): vol.All(vol.Coerce(int), vol.Range(min=1)),
    }
)


@callback
def async_setup_services(hass: HomeAssistant) -> None:
    async def list_prints(call: ServiceCall) -> ServiceResponse:
        reg = er.async_get(hass).async_get(call.data[ATTR_ENTITY_ID])
        entry = hass.config_entries.async_get_entry(reg.config_entry_id) if reg and reg.config_entry_id else None
        if reg is None or entry is None or entry.domain != DOMAIN or not hasattr(entry, "runtime_data"):
            raise ServiceValidationError(
                translation_domain=DOMAIN,
                translation_key="not_a_print_list",
                translation_placeholders={"entity_id": call.data[ATTR_ENTITY_ID]},
            )
        coordinator: AirSliceCoordinator = entry.runtime_data
        # <entry>_<printer>_prints
        printer_id = reg.unique_id.removeprefix(f"{entry.entry_id}_").removesuffix("_prints")
        # Fresh, not up to a minute old: this answers someone opening a list.
        await coordinator.async_refresh()
        found = select(coordinator.data.prints, printer_id, call.data.get(ATTR_SEARCH))
        limit = call.data.get(ATTR_LIMIT)
        return {"prints": found[:limit] if limit else found, "count": len(found)}

    hass.services.async_register(
        DOMAIN, "list_prints", list_prints, schema=LIST_SCHEMA, supports_response=SupportsResponse.ONLY
    )
