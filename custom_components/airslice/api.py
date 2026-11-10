"""Talks to AirSlice's HTTP API."""

from __future__ import annotations

from typing import Any

from aiohttp import ClientError, ClientSession, ClientTimeout

TIMEOUT = ClientTimeout(total=20)


class AirSliceError(Exception):
    """AirSlice can't be reached or answered with an error."""


class AirSliceAuthError(AirSliceError):
    """AirSlice refused the API key."""


class AirSliceTooOld(AirSliceError):
    """This AirSlice has no print history API yet."""


class AirSliceApi:
    def __init__(self, session: ClientSession, url: str, api_key: str) -> None:
        self._session = session
        self._url = url.rstrip("/")
        self._headers = {"Authorization": f"Bearer {api_key}"}

    async def _get(self, path: str) -> Any:
        try:
            async with self._session.get(
                f"{self._url}/api/{path}", headers=self._headers, timeout=TIMEOUT
            ) as res:
                if res.status in (401, 403):
                    raise AirSliceAuthError
                if res.status == 404 and path.startswith(("prints", "events")):
                    raise AirSliceTooOld
                if res.status >= 400:
                    raise AirSliceError(f"{path}: HTTP {res.status}")
                return await res.json()
        except (ClientError, TimeoutError, ValueError) as err:
            raise AirSliceError(f"{path}: {err}") from err

    async def info(self) -> dict[str, Any]:
        return await self._get("info")

    async def printers(self) -> list[dict[str, Any]]:
        return await self._get("printers")

    async def prints(self) -> list[dict[str, Any]]:
        return await self._get("prints")

    async def queue(self, printer_id: str) -> list[dict[str, Any]]:
        return await self._get(f"queue?printer_id={printer_id}")

    async def spools(self, printer_id: str) -> dict[str, Any]:
        return await self._get(f"printers/{printer_id}/spools")

    async def events(self, after: int | None) -> dict[str, Any]:
        return await self._get("events" if after is None else f"events?after={after}")

    async def _post(self, path: str, body: dict[str, Any] | None = None) -> None:
        try:
            async with self._session.post(
                f"{self._url}/api/{path}", headers=self._headers, json=body or {}, timeout=TIMEOUT
            ) as res:
                if res.status in (401, 403):
                    raise AirSliceAuthError
                if res.status >= 400:
                    text = await res.text()
                    raise AirSliceError(text[:300] or f"{path}: HTTP {res.status}")
        except (ClientError, TimeoutError) as err:
            raise AirSliceError(f"{path}: {err}") from err

    async def control(self, printer_id: str, action: str) -> None:
        await self._post(f"printers/{printer_id}/control", {"action": action})

    async def start_next(self, printer_id: str) -> None:
        await self._post(f"printers/{printer_id}/start-next")

    async def camera(self, printer_id: str) -> bytes | None:
        try:
            async with self._session.get(
                f"{self._url}/api/printers/{printer_id}/camera.jpg", headers=self._headers, timeout=TIMEOUT
            ) as res:
                return await res.read() if res.status == 200 else None
        except (ClientError, TimeoutError) as err:
            raise AirSliceError(f"camera: {err}") from err

    async def photo(self, job_id: str) -> bytes | None:
        """The camera picture of a print, if it has one."""
        try:
            async with self._session.get(
                f"{self._url}/api/jobs/{job_id}/photo.jpg",
                headers=self._headers,
                timeout=TIMEOUT,
            ) as res:
                if res.status != 200:
                    return None
                return await res.read()
        except (ClientError, TimeoutError) as err:
            raise AirSliceError(f"photo: {err}") from err
