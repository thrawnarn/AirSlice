"""AirSlice: the print history of a self-hosted AirSlice in Home Assistant.

AirSlice keeps the prints; this reads them, so nothing here stores history.
Each printer gets a sensor, `airslice.list_prints` gives cards the prints, the
result photos are served through the media source, and airslice-history-card
shows it all on a dashboard.
"""

from __future__ import annotations

from homeassistant.const import CONF_API_KEY, CONF_URL, Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers import config_validation as cv
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.typing import ConfigType

from .api import AirSliceApi
from .card import async_setup_card, async_unregister_resource
from .const import DOMAIN
from .coordinator import AirSliceConfigEntry, AirSliceCoordinator
from .media_source import PhotoView
from .services import async_setup_services

PLATFORMS: list[Platform] = [Platform.SENSOR]

CONFIG_SCHEMA = cv.config_entry_only_config_schema(DOMAIN)


async def async_setup(hass: HomeAssistant, config: ConfigType) -> bool:
    async_setup_services(hass)
    hass.http.register_view(PhotoView())
    await async_setup_card(hass)
    return True


async def async_setup_entry(hass: HomeAssistant, entry: AirSliceConfigEntry) -> bool:
    api = AirSliceApi(async_get_clientsession(hass), entry.data[CONF_URL], entry.data[CONF_API_KEY])
    coordinator = AirSliceCoordinator(hass, entry, api)
    await coordinator.async_config_entry_first_refresh()
    entry.runtime_data = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: AirSliceConfigEntry) -> bool:
    return await hass.config_entries.async_unload_platforms(entry, PLATFORMS)


async def async_remove_entry(hass: HomeAssistant, entry: AirSliceConfigEntry) -> None:
    """Take the card off the dashboards with the last AirSlice removed."""
    if not [e for e in hass.config_entries.async_entries(DOMAIN) if e.entry_id != entry.entry_id]:
        await async_unregister_resource(hass)
