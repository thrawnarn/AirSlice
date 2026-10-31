use crate::{
    config::Config,
    models::{ResolvedModel, SliceProgress},
    printer::PrinterManager,
    slicer::Slicer,
    store::Store,
};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;

pub struct AppState {
    pub cfg: Config,
    pub store: Store,
    pub printers: Arc<PrinterManager>,
    pub slicer: Slicer,
    pub http: reqwest::Client,
    resolve_cache: Mutex<HashMap<String, (Instant, ResolvedModel)>>,
    /// Slicing progress per job; in memory only, it changes twice a second.
    progress: std::sync::Mutex<HashMap<String, SliceProgress>>,
}

pub type Shared = Arc<AppState>;

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

impl AppState {
    pub fn new(cfg: Config, store: Store, printers: Arc<PrinterManager>, http: reqwest::Client) -> Self {
        let slicer = Slicer::new(cfg.clone());
        Self { cfg, store, printers, slicer, http, resolve_cache: Mutex::new(HashMap::new()), progress: Default::default() }
    }

    pub fn set_progress(&self, job: &str, p: Option<SliceProgress>) {
        let mut map = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        match p {
            Some(p) => map.insert(job.to_string(), p),
            None => map.remove(job),
        };
    }

    pub fn progress(&self, job: &str) -> Option<SliceProgress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).get(job).cloned()
    }

    pub async fn cached_model(&self, url: &str) -> Option<ResolvedModel> {
        let cache = self.resolve_cache.lock().await;
        cache.get(url).filter(|(t, _)| t.elapsed() < CACHE_TTL).map(|(_, m)| m.clone())
    }

    pub async fn cache_model(&self, url: &str, model: &ResolvedModel) {
        let mut cache = self.resolve_cache.lock().await;
        cache.retain(|_, (t, _)| t.elapsed() < CACHE_TTL);
        cache.insert(url.to_string(), (Instant::now(), model.clone()));
    }
}
