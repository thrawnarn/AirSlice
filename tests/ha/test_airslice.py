"""The integration on real Home Assistant, with AirSlice's HTTP API mocked."""

from homeassistant import config_entries
from homeassistant.components.media_source import async_resolve_media
from homeassistant.const import CONF_API_KEY, CONF_URL
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.exceptions import ServiceValidationError
from homeassistant.setup import async_setup_component
import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.airslice.const import DOMAIN

URL = "http://airslice.test:8080"
PRINTERS = [{"id": "p1", "name": "Pandan"}, {"id": "p2", "name": "Lilla"}]
PRINTS = [
    {
        "id": "abc123def456",
        "name": "Vase",
        "printer_id": "p1",
        "external": True,
        "outcome": "finished",
        "started_at": 1793610000,
        "ended_at": 1793613600,
        "progress": 100,
        "weight_g": None,
        "usage": [],
        "cost": {"filament": None, "machine": 1.5, "total": 1.5, "currency": "kr", "actual": True},
        "photo": True,
    },
    {
        "id": "000111222333",
        "name": "Egg, plate 2",
        "printer_id": "p1",
        "external": False,
        "outcome": "failed",
        "started_at": 1793600000,
        "ended_at": 1793603600,
        "progress": 25,
        "weight_g": 25.0,
        "usage": [{"slot": "A1", "type": "PLA", "color": "#FFFFFF", "grams": 25.0}],
        "waste": {"changes": 2, "flush_g": 1.0, "tower_g": 0.5, "cost": 0.2},
        "photo": False,
    },
    {
        "id": "999888777666",
        "name": "Other printer's",
        "printer_id": "p2",
        "outcome": "cancelled",
        "started_at": 1793500000,
        "ended_at": 1793500100,
        "photo": False,
    },
]


def mock_api(aioclient_mock, prints=PRINTS, status=200):
    aioclient_mock.get(f"{URL}/api/info", json={"version": "x"}, status=status)
    aioclient_mock.get(f"{URL}/api/printers", json=PRINTERS, status=status)
    aioclient_mock.get(f"{URL}/api/prints", json=prints, status=status)


async def setup(hass: HomeAssistant) -> MockConfigEntry:
    entry = MockConfigEntry(domain=DOMAIN, data={CONF_URL: URL, CONF_API_KEY: "as_key"}, unique_id=URL)
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


async def test_config_flow_creates_entry(hass, aioclient_mock):
    mock_api(aioclient_mock)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    assert result["type"] is FlowResultType.FORM
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_URL: f"{URL}/", CONF_API_KEY: "as_key"}
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"][CONF_URL] == URL
    # Every call carries the key.
    assert all(call[3]["Authorization"] == "Bearer as_key" for call in aioclient_mock.mock_calls)


@pytest.mark.parametrize(
    ("kwargs", "error"),
    [
        ({"status": 401}, "invalid_auth"),
        ({"status": 500}, "cannot_connect"),
    ],
)
async def test_config_flow_errors(hass, aioclient_mock, kwargs, error):
    mock_api(aioclient_mock, **kwargs)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_URL: URL, CONF_API_KEY: "as_key"}
    )
    assert result["errors"] == {"base": error}


async def test_config_flow_says_when_airslice_is_too_old(hass, aioclient_mock):
    aioclient_mock.get(f"{URL}/api/info", json={"version": "0.1.3"})
    aioclient_mock.get(f"{URL}/api/prints", status=404)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_URL: URL, CONF_API_KEY: "as_key"}
    )
    assert result["errors"] == {"base": "too_old"}


async def test_one_sensor_per_printer(hass, aioclient_mock):
    mock_api(aioclient_mock)
    await setup(hass)
    state = hass.states.get("sensor.airslice_pandan_prints")
    assert state.state == "2"
    assert state.attributes["latest_name"] == "Vase"
    assert state.attributes["latest_outcome"] == "finished"
    assert state.attributes["finished"] == 1
    assert state.attributes["hours"] == 2.0
    assert state.attributes["grams"] == 25.0
    assert state.attributes["cost"] == 1.5
    assert hass.states.get("sensor.airslice_lilla_prints").state == "1"


async def test_list_prints(hass, aioclient_mock):
    mock_api(aioclient_mock)
    entry = await setup(hass)
    res = await hass.services.async_call(
        DOMAIN, "list_prints", {"entity_id": "sensor.airslice_pandan_prints"}, blocking=True, return_response=True
    )
    assert res["count"] == 2
    vase, egg = res["prints"]
    assert vase["external"] is True
    assert vase["photo"] == f"media-source://{DOMAIN}/{entry.entry_id}/abc123def456"
    assert vase["started"] == "2026-11-02T09:00:00+00:00"
    assert egg["photo"] is None
    assert egg["usage"][0]["slot"] == "A1"
    assert egg["waste"]["flush_g"] == 1.0

    res = await hass.services.async_call(
        DOMAIN,
        "list_prints",
        {"entity_id": "sensor.airslice_pandan_prints", "search": "EGG plate"},
        blocking=True,
        return_response=True,
    )
    assert [p["name"] for p in res["prints"]] == ["Egg, plate 2"]
    res = await hass.services.async_call(
        DOMAIN,
        "list_prints",
        {"entity_id": "sensor.airslice_pandan_prints", "limit": 1},
        blocking=True,
        return_response=True,
    )
    assert len(res["prints"]) == 1
    assert res["count"] == 2


async def test_list_prints_refuses_other_entities(hass, aioclient_mock):
    mock_api(aioclient_mock)
    await setup(hass)
    with pytest.raises(ServiceValidationError):
        await hass.services.async_call(
            DOMAIN, "list_prints", {"entity_id": "sensor.not_this"}, blocking=True, return_response=True
        )


async def test_photo_goes_through_home_assistant(hass, aioclient_mock, hass_client, hass_client_no_auth):
    mock_api(aioclient_mock)
    aioclient_mock.get(f"{URL}/api/jobs/abc123def456/photo.jpg", content=b"\xff\xd8jpeg")
    aioclient_mock.get(f"{URL}/api/jobs/000111222333/photo.jpg", status=404)
    entry = await setup(hass)
    assert await async_setup_component(hass, "media_source", {})
    media = await async_resolve_media(hass, f"media-source://{DOMAIN}/{entry.entry_id}/abc123def456", None)
    assert media.mime_type == "image/jpeg"

    res = await (await hass_client()).get(media.url)
    assert res.status == 200
    assert await res.read() == b"\xff\xd8jpeg"
    # No login, no photo.
    assert (await (await hass_client_no_auth()).get(media.url)).status == 401
    client = await hass_client()
    assert (await client.get(f"/api/airslice/{entry.entry_id}/photo/000111222333")).status == 404
    # Home Assistant turns away a path that climbs out before it gets here.
    assert (await client.get(f"/api/airslice/{entry.entry_id}/photo/..%2Fkeys")).status == 400
    assert (await client.get(f"/api/airslice/{entry.entry_id}/photo/not-an-id")).status == 404
    assert (await client.get(f"/api/airslice/nope/photo/abc123def456")).status == 404


async def test_refused_key_asks_for_a_new_one(hass, aioclient_mock):
    mock_api(aioclient_mock, status=401)
    entry = await setup_failing(hass)
    assert entry.state is config_entries.ConfigEntryState.SETUP_ERROR
    flows = hass.config_entries.flow.async_progress_by_handler(DOMAIN)
    assert [f["context"]["source"] for f in flows] == [config_entries.SOURCE_REAUTH]


async def test_old_airslice_retries_setup(hass, aioclient_mock):
    aioclient_mock.get(f"{URL}/api/printers", json=PRINTERS)
    aioclient_mock.get(f"{URL}/api/prints", status=404)
    entry = await setup_failing(hass)
    assert entry.state is config_entries.ConfigEntryState.SETUP_RETRY


async def setup_failing(hass: HomeAssistant) -> MockConfigEntry:
    entry = MockConfigEntry(domain=DOMAIN, data={CONF_URL: URL, CONF_API_KEY: "as_key"}, unique_id=URL)
    entry.add_to_hass(hass)
    await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()
    return entry


async def test_card_is_served_and_kept_as_one_resource(hass, aioclient_mock, hass_client):
    from homeassistant.components.lovelace.const import LOVELACE_DATA

    from custom_components.airslice.card import CARD_URL

    assert await async_setup_component(hass, "lovelace", {})
    mock_api(aioclient_mock)
    resources = hass.data[LOVELACE_DATA].resources
    stale = await resources.async_create_item({"res_type": "module", "url": "/local/airslice-history-card.js?v=1"})
    other = await resources.async_create_item({"res_type": "module", "url": "/local/other-card.js"})

    entry = await setup(hass)
    await hass.async_block_till_done()
    urls = [i["url"] for i in resources.async_items()]
    ours = [u for u in urls if u.startswith(CARD_URL)]
    assert len(ours) == 1 and "?v=" in ours[0]
    assert other["url"] in urls
    assert stale["url"] not in urls

    res = await (await hass_client()).get(ours[0])
    assert res.status == 200
    assert "airslice-history-card" in await res.text()

    # Gone with the last AirSlice; what else was there stays.
    await hass.config_entries.async_remove(entry.entry_id)
    await hass.async_block_till_done()
    assert [i["url"] for i in resources.async_items()] == [other["url"]]
