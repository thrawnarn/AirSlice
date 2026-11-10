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
    aioclient_mock.get(f"{URL}/api/events", json={"latest": 5, "events": []}, status=status)
    for printer in PRINTERS:
        aioclient_mock.get(f"{URL}/api/queue?printer_id={printer['id']}", json=[], status=status)
        aioclient_mock.get(f"{URL}/api/printers/{printer['id']}/spools", json={"configured": False}, status=status)


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


LIVE_PRINTER = {
    "id": "p1",
    "name": "Pandan",
    "printer_model": "Bambu Lab P1S",
    "camera": "printer",
    "current_job": {"id": "j1", "started_at": 1793610000, "weight_g": 89.3, "length_m": 29.5},
    "status": {
        "connected": True,
        "state": "PAUSE",
        "task_name": "Vase_ab12cd",
        "percent": 63,
        "remaining_minutes": 42,
        "layer": 80,
        "total_layers": 125,
        "nozzle_temp": 219.5,
        "nozzle_target": 220,
        "bed_temp": 60,
        "bed_target": 60,
        "chamber_light": True,
        "speed_level": 2,
        "stage": 6,
        "wifi_dbm": -52,
        "fan_part": 80,
        "print_error": None,
        "ams_backup": True,
        "hms": ["1800_2000_0002_0001"],
        "problems": [
            {
                "code": "1800_2000_0002_0001",
                "severity": "serious",
                "text": "Out of filament",
                "texts": {"en": "Out of filament", "sv": "Slut på filament"},
                "url": "https://wiki.example/x",
            }
        ],
        "trays": [
            {"id": 0, "ams": 0, "slot": 0, "type": "PLA", "color": "#FFFFFF", "name": "Bambu PLA Matte", "remain": None, "active": False},
            {"id": 1, "ams": 0, "slot": 1, "type": "", "color": "", "remain": None, "active": False},
            {"id": 128, "ams": 128, "slot": 0, "type": "ABS", "color": "#222222", "name": None, "remain": 40, "spool_g": 1000, "active": True},
        ],
        "external": {"id": 254, "type": "TPU", "color": "#000000", "name": None, "remain": None, "active": False},
        "ams_units": [{"id": 0, "name": "AMS A", "humidity_pct": 35, "humidity_level": 3, "temp": 19.6, "dry_minutes": None}],
    },
}
QUEUED = {"id": "q1", "title": "Next one", "plate_label": "Plate 2", "prediction_secs": 3600, "weight_g": 40.0, "filament_ok": True}


def mock_live(aioclient_mock, printer=LIVE_PRINTER, queue=(), events=()):
    aioclient_mock.clear_requests()
    aioclient_mock.get(f"{URL}/api/info", json={"version": "x"})
    aioclient_mock.get(f"{URL}/api/printers", json=[printer])
    aioclient_mock.get(f"{URL}/api/prints", json=PRINTS[:2])
    aioclient_mock.get(f"{URL}/api/queue?printer_id=p1", json=list(queue))
    aioclient_mock.get(f"{URL}/api/printers/p1/spools", json={"configured": True, "trays": {"0": {"id": 7, "remaining_g": 863.4}}})
    # The more specific first: a mock without a query matches any query.
    aioclient_mock.get(f"{URL}/api/events?after=5", json={"latest": 6, "events": list(events)})
    aioclient_mock.get(f"{URL}/api/events", json={"latest": 5, "events": []})
    aioclient_mock.post(f"{URL}/api/printers/p1/control", json={"ok": True})
    aioclient_mock.post(f"{URL}/api/printers/p1/start-next", json={})
    aioclient_mock.get(f"{URL}/api/printers/p1/camera.jpg", content=b"jpeg-bytes")


async def refresh(hass, entry):
    await entry.runtime_data.live.async_refresh()
    await hass.async_block_till_done()


def control_calls(aioclient_mock):
    return [c[2]["action"] for c in aioclient_mock.mock_calls if str(c[1]).endswith("/control")]


async def test_live_sensors_follow_the_printer(hass, aioclient_mock):
    mock_live(aioclient_mock)
    await setup(hass)
    for entity, state in {
        "sensor.airslice_pandan_print_status": "pause",
        "sensor.airslice_pandan_task_name": "Vase_ab12cd",
        "sensor.airslice_pandan_print_progress": "63",
        "sensor.airslice_pandan_remaining_time": "42",
        "sensor.airslice_pandan_current_layer": "80",
        "sensor.airslice_pandan_total_layer_count": "125",
        "sensor.airslice_pandan_nozzle_temperature": "219.5",
        "sensor.airslice_pandan_current_stage": "paused_filament_ran_out",
        "sensor.airslice_pandan_active_tray": "AMS HT 1",
        "sensor.airslice_pandan_print_weight": "89.3",
        "sensor.airslice_pandan_ams_0_humidity": "35",
        "binary_sensor.airslice_pandan_online": "on",
        "binary_sensor.airslice_pandan_hms_errors": "on",
        "binary_sensor.airslice_pandan_print_error": "off",
        "binary_sensor.airslice_pandan_ams_drying": "off",
        "light.airslice_pandan_chamber_light": "on",
        "select.airslice_pandan_printing_speed": "standard",
    }.items():
        assert hass.states.get(entity).state == state, entity
    assert hass.states.get("sensor.airslice_pandan_start_time").state.startswith("2026-")
    assert hass.states.get("sensor.airslice_pandan_end_time").state.startswith("20")
    hms = hass.states.get("binary_sensor.airslice_pandan_hms_errors").attributes
    assert hms["1-Code"] == "HMS_1800_2000_0002_0001" and hms["1-Error"] == "Out of filament"


async def test_trays_show_what_is_loaded_and_how_much(hass, aioclient_mock):
    mock_live(aioclient_mock)
    await setup(hass)
    tray = hass.states.get("sensor.airslice_pandan_tray_0")
    assert tray.state == "Bambu PLA Matte"
    assert tray.attributes["tray"] == "AMS A1"
    assert tray.attributes["remaining_g"] == 863.4  # Spoolman first
    assert hass.states.get("sensor.airslice_pandan_tray_1").state == "empty"
    ht = hass.states.get("sensor.airslice_pandan_tray_128")
    assert ht.state == "ABS" and ht.attributes["remaining_g"] == 400 and ht.attributes["active"] is True
    assert hass.states.get("sensor.airslice_pandan_external_spool").state == "TPU"


async def test_a_printer_that_is_off_says_offline_and_the_rest_steps_aside(hass, aioclient_mock):
    mock_live(aioclient_mock)
    entry = await setup(hass)
    off = {**LIVE_PRINTER, "status": {**LIVE_PRINTER["status"], "connected": False}}
    mock_live(aioclient_mock, off)
    await refresh(hass, entry)
    assert hass.states.get("sensor.airslice_pandan_print_status").state == "offline"
    assert hass.states.get("binary_sensor.airslice_pandan_online").state == "off"
    assert hass.states.get("sensor.airslice_pandan_nozzle_temperature").state == "unavailable"


async def test_controls_reach_the_printer(hass, aioclient_mock):
    mock_live(aioclient_mock)
    await setup(hass)
    assert hass.states.get("button.airslice_pandan_pause_printing").state == "unavailable"  # already paused
    await hass.services.async_call("button", "press", {"entity_id": "button.airslice_pandan_resume_printing"}, blocking=True)
    await hass.services.async_call("light", "turn_off", {"entity_id": "light.airslice_pandan_chamber_light"}, blocking=True)
    await hass.services.async_call("select", "select_option", {"entity_id": "select.airslice_pandan_printing_speed", "option": "sport"}, blocking=True)
    assert control_calls(aioclient_mock) == ["resume", "light_off", "speed_3"]


async def test_queue_sensor_and_start_next(hass, aioclient_mock):
    idle = {**LIVE_PRINTER, "status": {**LIVE_PRINTER["status"], "state": "IDLE"}}
    mock_live(aioclient_mock, idle, queue=[QUEUED])
    await setup(hass)
    queue = hass.states.get("sensor.airslice_pandan_queue")
    assert queue.state == "1"
    assert queue.attributes["prints"][0] == {"title": "Next one", "plate": "Plate 2", "minutes": 60, "weight": 40.0, "filament_ok": True}
    await hass.services.async_call("button", "press", {"entity_id": "button.airslice_pandan_start_next"}, blocking=True)
    assert any(str(c[1]).endswith("/start-next") for c in aioclient_mock.mock_calls)


async def test_start_next_needs_a_queue_and_a_free_printer(hass, aioclient_mock):
    mock_live(aioclient_mock, queue=[QUEUED])  # the printer is paused
    await setup(hass)
    assert hass.states.get("button.airslice_pandan_start_next").state == "unavailable"


async def test_camera_gives_the_picture(hass, aioclient_mock):
    from homeassistant.components.camera import async_get_image

    mock_live(aioclient_mock)
    await setup(hass)
    image = await async_get_image(hass, "camera.airslice_pandan_camera")
    assert image.content == b"jpeg-bytes"


async def test_events_reach_automations_once(hass, aioclient_mock):
    fired = []
    hass.bus.async_listen("airslice_event", fired.append)
    mock_live(aioclient_mock)
    entry = await setup(hass)
    assert not fired, "what happened before Home Assistant looked isn't replayed"
    event = {"seq": 6, "at": 1, "kind": "paused", "printer_id": "p1", "job_id": "j1", "title": "Vase", "message": "Out of filament"}
    mock_live(aioclient_mock, events=[event])
    await refresh(hass, entry)
    assert [(e.data["kind"], e.data["printer"]) for e in fired] == [("paused", "Pandan")]
    state = hass.states.get("event.airslice_pandan_event")
    assert state.attributes["event_type"] == "paused" and state.attributes["message"] == "Out of filament"


async def test_an_older_airslice_without_events_still_works(hass, aioclient_mock):
    mock_live(aioclient_mock)
    aioclient_mock.clear_requests()
    aioclient_mock.get(f"{URL}/api/info", json={"version": "x"})
    aioclient_mock.get(f"{URL}/api/printers", json=[LIVE_PRINTER])
    aioclient_mock.get(f"{URL}/api/prints", json=[])
    aioclient_mock.get(f"{URL}/api/queue?printer_id=p1", json=[])
    aioclient_mock.get(f"{URL}/api/printers/p1/spools", status=502)
    aioclient_mock.get(f"{URL}/api/events", status=404)
    await setup(hass)
    assert hass.states.get("sensor.airslice_pandan_print_status").state == "pause"
