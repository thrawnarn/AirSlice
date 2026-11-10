"""The history card: served by the integration, and kept on the dashboards.

The card is built into www/, so it travels with the integration. A dashboard
only loads it through a resource, so the integration keeps exactly one: versioned
by the built file's content, so a new card is a new URL, and removed again with
the last AirSlice entry. Dashboards in YAML mode have resources only their owner
can change; the log says which URL to add.
"""

from __future__ import annotations

import hashlib
import logging
from pathlib import Path
from typing import Any

from homeassistant.components.http import StaticPathConfig
from homeassistant.core import HomeAssistant
from homeassistant.helpers.start import async_at_started

from .const import DOMAIN

_LOGGER = logging.getLogger(__name__)

CARD_FILE = "airslice-history-card.js"
URL_BASE = f"/{DOMAIN}/frontend"
CARD_URL = f"{URL_BASE}/{CARD_FILE}"
WWW = Path(__file__).parent / "www"
RESOURCE_TYPE = "module"


async def async_setup_card(hass: HomeAssistant) -> None:
    """Serve www/, and once Home Assistant is up, keep the resource on it."""
    # Not cached by path: the resource's ?v= is what browsers cache by.
    await hass.http.async_register_static_paths([StaticPathConfig(URL_BASE, str(WWW), False)])
    version = await hass.async_add_executor_job(card_version, WWW / CARD_FILE)
    url = f"{CARD_URL}?v={version}"

    async def _register(hass: HomeAssistant) -> None:
        await _async_register_resource(hass, url)

    # Lovelace has its resources by then, whatever order things came up in.
    async_at_started(hass, _register)


def card_version(path: Path) -> str:
    """A short fingerprint of the built card, for its URL."""
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()[:10]
    except OSError:
        _LOGGER.warning("The card is missing from %s; was it built?", path)
        return "0"


def _resources(hass: HomeAssistant) -> Any | None:
    """Lovelace's resource collection, if the integration may change it."""
    from homeassistant.components.lovelace.const import LOVELACE_DATA

    lovelace = hass.data.get(LOVELACE_DATA)
    if lovelace is None:
        return None
    # Called resource_mode since 2026.2, and mode before.
    if getattr(lovelace, "resource_mode", getattr(lovelace, "mode", None)) != "storage":
        _LOGGER.info("Dashboards are in YAML mode: add resource %s, type module, for the history card", CARD_URL)
        return None
    return lovelace.resources


async def _async_loaded(resources: Any) -> None:
    """Writing before Lovelace has read the stored list would save over it."""
    if not getattr(resources, "loaded", True):
        await resources.async_load()
        resources.loaded = True


def _path(url: str) -> str:
    return str(url).split("?", 1)[0]


async def _async_sync(resources: Any, url: str) -> None:
    """Bring the collection to one resource for the card, at `url`."""
    cards = [i for i in resources.async_items() if _path(i.get("url", "")).endswith(f"/{CARD_FILE}")]
    if not cards:
        await resources.async_create_item({"res_type": RESOURCE_TYPE, "url": url})
        return
    # Ours first; any other copy, such as one from /local, becomes ours.
    cards.sort(key=lambda i: _path(i["url"]) != CARD_URL)
    for extra in cards[1:]:
        await resources.async_delete_item(extra["id"])
    if cards[0].get("url") != url:
        await resources.async_update_item(cards[0]["id"], {"res_type": RESOURCE_TYPE, "url": url})


async def _async_register_resource(hass: HomeAssistant, url: str) -> None:
    resources = _resources(hass)
    if resources is None:
        return
    try:
        await _async_loaded(resources)
        await _async_sync(resources, url)
    except Exception:  # noqa: BLE001 - a resource is never worth failing setup
        _LOGGER.warning("Couldn't add the history card as a dashboard resource; add %s (type module)", url, exc_info=True)


async def async_unregister_resource(hass: HomeAssistant) -> None:
    """Take the card's resource away, with the last of the integration."""
    resources = _resources(hass)
    if resources is None:
        return
    try:
        await _async_loaded(resources)
        for item in list(resources.async_items()):
            if _path(item.get("url", "")) == CARD_URL:
                await resources.async_delete_item(item["id"])
    except Exception:  # noqa: BLE001
        _LOGGER.warning("Couldn't remove the history card's resource", exc_info=True)
