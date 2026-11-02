"""Result photos: browsable as media, served to whoever Home Assistant lets in."""

from __future__ import annotations

from http import HTTPStatus

from aiohttp import web

from homeassistant.components.http import HomeAssistantView
from homeassistant.components.media_player import BrowseError, MediaClass
from homeassistant.components.media_source import (
    BrowseMediaSource,
    MediaSource,
    MediaSourceItem,
    PlayMedia,
    Unresolvable,
)
from homeassistant.core import HomeAssistant

from .api import AirSliceError
from .const import DOMAIN

JPEG = "image/jpeg"


async def async_get_media_source(hass: HomeAssistant) -> MediaSource:
    return AirSliceMediaSource(hass)


class AirSliceMediaSource(MediaSource):
    name = "AirSlice"

    def __init__(self, hass: HomeAssistant) -> None:
        super().__init__(DOMAIN)
        self.hass = hass

    def _loaded(self, entry_id: str):
        entry = self.hass.config_entries.async_get_entry(entry_id)
        if entry is None or entry.domain != DOMAIN or not hasattr(entry, "runtime_data"):
            return None
        return entry

    async def async_resolve_media(self, item: MediaSourceItem) -> PlayMedia:
        entry_id, _, job_id = (item.identifier or "").partition("/")
        if not job_id or self._loaded(entry_id) is None:
            raise Unresolvable(f"No such photo: {item.identifier}")
        return PlayMedia(f"/api/airslice/{entry_id}/photo/{job_id}", JPEG)

    def _node(self, identifier: str, title: str, *, photo: bool = False, children=None) -> BrowseMediaSource:
        return BrowseMediaSource(
            domain=DOMAIN,
            identifier=identifier or None,
            media_class=MediaClass.IMAGE if photo else MediaClass.DIRECTORY,
            media_content_type=JPEG if photo else "",
            title=title,
            can_play=photo,
            can_expand=not photo,
            children=children,
        )

    async def async_browse_media(self, item: MediaSourceItem) -> BrowseMediaSource:
        if item.identifier:
            raise BrowseError("A photo has nothing inside it")
        children = []
        for entry in self.hass.config_entries.async_loaded_entries(DOMAIN):
            for p in entry.runtime_data.data.prints:
                if p["photo"]:
                    children.append(
                        self._node(f"{entry.entry_id}/{p['id']}", f"{p['name']} ({p['ended'][:10]})", photo=True)
                    )
        return self._node("", "AirSlice", children=children)


class PhotoView(HomeAssistantView):
    """Needs a logged-in user or a signed link, which resolve_media gives."""

    url = "/api/airslice/{entry_id}/photo/{job_id}"
    name = "api:airslice:photo"

    async def get(self, request: web.Request, entry_id: str, job_id: str) -> web.Response:
        hass: HomeAssistant = request.app["hass"]
        entry = hass.config_entries.async_get_entry(entry_id)
        if entry is None or entry.domain != DOMAIN or not hasattr(entry, "runtime_data"):
            return web.Response(status=HTTPStatus.NOT_FOUND)
        if not job_id.isalnum():
            return web.Response(status=HTTPStatus.NOT_FOUND)
        try:
            photo = await entry.runtime_data.api.photo(job_id)
        except AirSliceError:
            return web.Response(status=HTTPStatus.BAD_GATEWAY)
        if photo is None:
            return web.Response(status=HTTPStatus.NOT_FOUND)
        return web.Response(body=photo, content_type=JPEG, headers={"Cache-Control": "private, max-age=86400"})
