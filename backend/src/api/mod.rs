mod auth;
pub(crate) mod jobs;
mod printers;
mod settings;

use crate::state::Shared;
use axum::{
    Router,
    middleware::from_fn_with_state,
    routing::{delete, get, post},
};

pub fn router(state: Shared) -> Router<Shared> {
    let public = Router::new()
        .route("/health", get(settings::health))
        .route("/auth/status", get(auth::status))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/notify/picture/{id}", get(settings::notify_picture));

    let protected = Router::new()
        .route("/auth/password", post(auth::change_password))
        .route("/auth/sessions", delete(auth::revoke_sessions))
        .route("/keys", get(auth::list_keys).post(auth::create_key))
        .route("/keys/{id}", delete(auth::delete_key))
        .route("/info", get(settings::info))
        .route("/diagnostics", get(settings::diagnostics))
        .route("/settings/connection", get(settings::connection).put(settings::set_connection))
        .route("/settings/notify", get(settings::notify).put(settings::set_notify))
        .route("/settings/notify/services", post(settings::notify_services))
        .route("/settings/notify/test", post(settings::notify_test))
        .route("/settings/notify/webpush", get(settings::webpush_key).post(settings::webpush_subscribe))
        .route("/settings/spoolman", get(settings::spoolman).put(settings::set_spoolman))
        .route("/settings/spoolman/test", post(settings::spoolman_test))
        .route("/settings/cost", get(settings::cost).put(settings::set_cost))
        .route("/settings/storage", get(settings::storage).put(settings::set_storage))
        .route("/settings/storage/clean", post(settings::clean_storage))
        .route("/profiles/machines", get(settings::machines))
        .route("/profiles", get(settings::profiles))
        .route("/bambu", get(settings::bambu_status).delete(settings::bambu_unlink))
        .route("/settings/presets", get(settings::presets).put(settings::set_presets))
        .route(
            "/settings/thingiverse",
            get(settings::thingiverse).put(settings::set_thingiverse).delete(settings::remove_thingiverse),
        )
        .route("/bambu/login", post(settings::bambu_login))
        .route("/bambu/verify", post(settings::bambu_verify))
        .route("/bambu/token", post(settings::bambu_token))
        .route("/printers", get(printers::list).post(printers::create))
        .route("/plates", get(printers::plates))
        .route("/printers/{id}", get(printers::get).put(printers::update).delete(printers::delete))
        .route("/printers/{id}/test", post(printers::test))
        .route("/printers/{id}/control", post(printers::control))
        .route("/printers/{id}/default", post(printers::make_default))
        .route("/resolve", post(jobs::resolve))
        .route("/makerworld/collections", get(jobs::collections))
        .route("/makerworld/image", get(jobs::makerworld_image))
        .route("/makerworld/collections/{id}", get(jobs::collection))
        .route("/jobs", get(jobs::list).post(jobs::create))
        .route(
            "/jobs/upload",
            post(jobs::upload).layer(axum::extract::DefaultBodyLimit::max(crate::sources::MAX_DOWNLOAD as usize)),
        )
        .route("/jobs/next-plate", get(jobs::next_plate_suggestion))
        .route("/jobs/{id}/extras", post(jobs::add_extra))
        .route(
            "/jobs/{id}/extras/upload",
            post(jobs::upload_extra).layer(axum::extract::DefaultBodyLimit::max(crate::sources::MAX_DOWNLOAD as usize)),
        )
        .route("/jobs/{id}/extras/{extra}", axum::routing::delete(jobs::remove_extra))
        .route("/jobs/{id}", get(jobs::get).delete(jobs::delete))
        .route("/jobs/{id}/slice", post(jobs::slice))
        .route("/jobs/{id}/slice-all", post(jobs::slice_all))
        .route("/jobs/{id}/print", post(jobs::print))
        .route("/jobs/{id}/queue", post(jobs::enqueue))
        .route("/jobs/{id}/queue-next-plate", post(jobs::queue_next_plate))
        .route("/jobs/{id}/favorite", post(jobs::favorite))
        .route("/jobs/{id}/reprint", post(jobs::reprint))
        .route("/jobs/{id}/reslice", post(jobs::reslice))
        .route("/jobs/{id}/cancel-wait", post(jobs::cancel_wait))
        .route("/queue", get(jobs::queue))
        .route("/queue/{id}", delete(jobs::dequeue))
        .route("/queue/{id}/move", post(jobs::move_item))
        .route("/queue/{id}/start", post(jobs::start_queued))
        .route("/queue/{id}/reslice", post(jobs::reslice_queued))
        .route("/printers/{id}/start-next", post(jobs::start_next))
        .route("/printers/{id}/tray-filaments", get(printers::tray_filaments))
        .route("/printers/{id}/trays/{tray}", post(printers::set_tray))
        .route("/printers/{id}/trays/{tray}/reread", post(printers::reread_tray))
        .route("/printers/{id}/trays/{tray}/spool", post(printers::link_spool))
        .route("/printers/{id}/ams/{unit}/dry", post(printers::start_drying).delete(printers::stop_drying))
        .route("/printers/{id}/spools", get(printers::spools))
        .route("/printers/{id}/camera.jpg", get(printers::camera))
        .route("/ha/cameras", get(printers::ha_cameras))
        .route("/ha/switches", get(printers::ha_switches))
        .route("/printers/{id}/power", post(printers::power))
        .route("/jobs/{id}/photo.jpg", get(jobs::photo).put(jobs::set_photo))
        .route("/prints", get(jobs::prints))
        .route("/prints/import", post(jobs::import_print))
        .route("/jobs/{id}/layers", get(jobs::layers))
        .route("/jobs/{id}/layers/{n}", get(jobs::layer))
        .route("/jobs/{id}/plates/{plate}/thumbnail", get(jobs::thumbnail))
        .route("/jobs/{id}/preview", get(jobs::preview))
        .route("/jobs/{id}/tray-suggestion", get(jobs::tray_suggestion))
        .route("/print", post(jobs::quick_print))
        .layer(from_fn_with_state(state, crate::auth::require_auth));

    public.merge(protected)
}
