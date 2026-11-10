"""Constants for AirSlice."""

DOMAIN = "airslice"

ATTR_LIMIT = "limit"
ATTR_SEARCH = "search"

SCAN_INTERVAL_SECONDS = 60
LIVE_INTERVAL_SECONDS = 10

EVENT_AIRSLICE = "airslice_event"
EVENT_TYPES = [
    "print_started",
    "first_layer",
    "almost_done",
    "print_finished",
    "print_failed",
    "print_stopped",
    "job_failed",
    "paused",
    "printer_problem",
    "filament_low",
    "contact_lost",
    "contact_back",
    "queue_ready",
]
