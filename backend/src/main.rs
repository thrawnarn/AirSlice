mod api;
mod attention;
mod auth;
mod camera;
mod bambu_cloud;
mod colors;
mod config;
mod diagnostics;
mod error;
mod events;
mod filament_keys;
mod gcode;
mod i18n;
mod notify;
mod power;
mod jobs;
mod preview;
mod prints;
mod mcp;
mod models;
mod printer;
mod slicer;
mod spoolman;
mod sources;
mod state;
mod store;
mod threemf;
mod tls;
mod webpush;

use axum::{
    Router,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Redirect, Response},
};
use std::sync::Arc;
use tower_http::{services::ServeDir, trace::TraceLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    {
        use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn,rumqttc=warn".into()))
            .with(fmt::layer().with_ansi(std::env::var_os("NO_COLOR").is_none()))
            // The last lines, for Settings → About → Copy diagnostics.
            .with(fmt::layer().with_ansi(false).with_writer(diagnostics::LogBuffer))
            .init();
    }
    tls::install_default_provider();

    let cfg = config::Config::from_env()?;
    tokio::fs::create_dir_all(cfg.jobs_dir()).await?;
    let store = store::Store::open(&cfg.data_dir.join("state.json")).await?;
    let http = reqwest::Client::builder()
        .user_agent(sources::USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let printers = printer::PrinterManager::new();
    let state = Arc::new(state::AppState::new(cfg.clone(), store, printers.clone(), http));

    printers.set_settings(state.store.read(|d| d.connection).await);
    jobs::recover(&state).await?;
    tokio::spawn(jobs::watch_prints(state.clone()));
    tokio::spawn(attention::watch(state.clone()));
    tokio::spawn(jobs::clean_files_hourly(state.clone()));
    tokio::spawn(power::watch(state.clone()));
    tokio::spawn(printer::hms::keep_fresh(state.clone()));
    printers.sync(&state.store.read(|d| d.printers.clone()).await).await;
    // Warm the profile index in the background (it reads a few thousand files).
    {
        let st = state.clone();
        tokio::spawn(async move {
            match st.slicer.profiles().await {
                Ok(idx) => tracing::info!("loaded {} OrcaSlicer printer profiles", idx.machines().len()),
                Err(e) => tracing::warn!("OrcaSlicer profiles unavailable: {e:#}"),
            }
        });
    }
    if !state.slicer.available() {
        tracing::warn!("OrcaSlicer not found at {} — slicing will fail", cfg.orca_path.display());
    }

    let static_files = ServeDir::new(&cfg.static_dir).append_index_html_on_directories(true);
    let app = Router::new()
        .nest("/api", api::router(state.clone()).fallback(api_not_found))
        .fallback_service(static_files)
        .layer(axum::middleware::from_fn(security_headers))
        .with_state(state.clone());

    let app = if cfg.base_path.is_empty() {
        app
    } else {
        let base = cfg.base_path.clone();
        Router::new()
            .nest_service(&base, app)
            .fallback(|uri: Uri| async move { (StatusCode::NOT_FOUND, format!("not found: {uri}")) })
            // "/airslice" -> "/airslice/" so relative asset URLs resolve.
            .layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
                let base = base.clone();
                async move {
                    if req.uri().path() == base {
                        let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
                        return Redirect::permanent(&format!("{base}/{query}")).into_response();
                    }
                    next.run(req).await
                }
            }))
    };
    let app = app
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth::mark_ingress))
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    tracing::info!(
        "AirSlice listening on http://{}{}/ (data: {})",
        cfg.bind,
        cfg.base_path,
        cfg.data_dir.display()
    );
    if let Some(peer) = cfg.ingress_peer {
        tracing::info!("Home Assistant ingress enabled (trusted peer {peer})");
    }
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error": "no such endpoint"})))
}

async fn security_headers(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let is_api = req.uri().path().contains("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert("x-content-type-options", header::HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", header::HeaderValue::from_static("same-origin"));
    // Through Home Assistant ingress these pages share Home Assistant's
    // origin, so injected script could act as the signed-in admin. Only
    // AirSlice's own scripts may run; images may come from the model sites.
    h.insert(
        "content-security-policy",
        header::HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             font-src 'self' https://fonts.gstatic.com; img-src 'self' data: blob: https:; connect-src 'self'; \
             worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
             frame-ancestors 'self'",
        ),
    );
    if is_api && !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    }
    res
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
