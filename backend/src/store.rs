//! State: settings, printers and keys in a JSON file (`state.json`), the job
//! history in SQLite (`airslice.db`, one row per job). Everything is kept in
//! memory; every mutation is written before returning, and only what changed:
//! the JSON file when a setting changed, the job rows that changed.

use crate::models::{
    ApiKey, BambuAccount, ConnectionSettings, CostSettings, Job, NotifySettings, Printer, QueueItem, Session, SpoolmanSettings, StorageSettings,
};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub version: u32,
    pub admin_password_hash: Option<String>,
    pub sessions: Vec<Session>,
    pub api_keys: Vec<ApiKey>,
    pub printers: Vec<Printer>,
    pub default_printer_id: Option<String>,
    pub bambu: BambuAccount,
    /// Thingiverse app token (thingiverse.com/developers).
    #[serde(default)]
    pub thingiverse_token: Option<String>,
    /// Saved slice settings; `None` until changed (then the defaults show).
    #[serde(default)]
    pub slice_presets: Option<Vec<crate::models::SlicePreset>>,
    /// Newest first. Lives in the database; read from state.json only to
    /// move the history of pre-alpha builds over.
    #[serde(skip_serializing)]
    pub jobs: Vec<Job>,
    pub queue: Vec<QueueItem>,
    /// Retries and waiting for offline printers.
    #[serde(default)]
    pub connection: ConnectionSettings,
    #[serde(default)]
    pub notify: NotifySettings,
    #[serde(default)]
    pub spoolman: SpoolmanSettings,
    #[serde(default)]
    pub cost: CostSettings,
    #[serde(default)]
    pub storage: StorageSettings,
    /// VAPID signing key for browser notifications (PKCS#8, base64url).
    #[serde(default)]
    pub webpush_key: Option<String>,
}

pub struct Store {
    path: PathBuf,
    data: RwLock<Data>,
    /// What is on disk: state.json's bytes and each job's JSON row.
    written: Mutex<Written>,
    db: Arc<std::sync::Mutex<rusqlite::Connection>>,
}

#[derive(Default)]
struct Written {
    state: Vec<u8>,
    jobs: HashMap<String, String>,
}

impl Store {
    /// `path` is state.json; the database sits next to it.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let mut data = match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice::<Data>(&bytes)
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Data { version: 1, ..Default::default() },
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        data.notify.migrate();
        let db_path = path.with_file_name("airslice.db");
        let (db, rows) = tokio::task::spawn_blocking(move || open_db(&db_path)).await??;
        let mut written = Written::default();
        if rows.is_empty() {
            // First start on the database: the jobs (if any) came from state.json
            // and are written below, before state.json is rewritten without them.
            if !data.jobs.is_empty() {
                tracing::info!("moving {} jobs from state.json to the database", data.jobs.len());
            }
        } else {
            let mut jobs = Vec::with_capacity(rows.len());
            for (id, json) in rows {
                match serde_json::from_str::<Job>(&json) {
                    Ok(j) => {
                        jobs.push(j);
                        written.jobs.insert(id, json);
                    }
                    // Keep the row; a later version may read it.
                    Err(e) => tracing::warn!("skipping job {id} from the database: {e}"),
                }
            }
            data.jobs = jobs;
        }
        let store = Self { path: path.to_path_buf(), data: RwLock::new(data), written: Mutex::new(written), db: Arc::new(std::sync::Mutex::new(db)) };
        store.update(|_| ()).await?; // make sure both exist and are writable
        Ok(store)
    }

    pub async fn read<R>(&self, f: impl FnOnce(&Data) -> R) -> R {
        let guard = self.data.read().await;
        f(&guard)
    }

    /// Mutate the state and persist it. Writes are serialized so what is on
    /// disk always reflects the latest state.
    pub async fn update<R>(&self, f: impl FnOnce(&mut Data) -> R) -> anyhow::Result<R> {
        let mut written = self.written.lock().await;
        let (result, state, jobs) = {
            let mut guard = self.data.write().await;
            let result = f(&mut guard);
            // Serializing reads only: let readers in meanwhile (writers are
            // already one at a time through `written`).
            let guard = guard.downgrade();
            let jobs = guard
                .jobs
                .iter()
                .map(|j| Ok((j.id.clone(), j.created_at, serde_json::to_string(j)?)))
                .collect::<anyhow::Result<Vec<_>>>()?;
            (result, serde_json::to_vec_pretty(&*guard)?, jobs)
        };
        let changed: Vec<(String, i64, String)> =
            jobs.iter().filter(|(id, _, json)| written.jobs.get(id) != Some(json)).cloned().collect();
        let present: std::collections::HashSet<&str> = jobs.iter().map(|(id, _, _)| id.as_str()).collect();
        let removed: Vec<String> = written.jobs.keys().filter(|id| !present.contains(id.as_str())).cloned().collect();
        if !changed.is_empty() || !removed.is_empty() {
            let db = self.db.clone();
            let (c, r) = (changed.clone(), removed.clone());
            tokio::task::spawn_blocking(move || write_jobs(&db, &c, &r)).await??;
            for id in &removed {
                written.jobs.remove(id);
            }
            for (id, _, json) in changed {
                written.jobs.insert(id, json);
            }
        }
        if state != written.state {
            write_atomic(&self.path, &state).await?;
            written.state = state;
        }
        Ok(result)
    }
}

/// Open (or create) the database and read the jobs, newest first.
fn open_db(path: &Path) -> anyhow::Result<(rusqlite::Connection, Vec<(String, String)>)> {
    let db = rusqlite::Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         CREATE TABLE IF NOT EXISTS jobs (
             id TEXT PRIMARY KEY,
             created_at INTEGER NOT NULL,
             data TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS jobs_created ON jobs (created_at);",
    )?;
    let rows = {
        let mut q = db.prepare("SELECT id, data FROM jobs ORDER BY created_at DESC, rowid DESC")?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?
    };
    Ok((db, rows))
}

fn write_jobs(db: &std::sync::Mutex<rusqlite::Connection>, changed: &[(String, i64, String)], removed: &[String]) -> anyhow::Result<()> {
    let mut db = db.lock().unwrap_or_else(|e| e.into_inner());
    let tx = db.transaction()?;
    {
        let mut up = tx.prepare_cached(
            "INSERT INTO jobs (id, created_at, data) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET created_at = excluded.created_at, data = excluded.data",
        )?;
        for (id, created, json) in changed {
            up.execute(rusqlite::params![id, created, json])?;
        }
        let mut del = tx.prepare_cached("DELETE FROM jobs WHERE id = ?1")?;
        for id in removed {
            del.execute([id])?;
        }
    }
    tx.commit()?;
    // Keep the main file complete, so a file-level backup (Home Assistant
    // copies the data folder while running) holds every job.
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, bytes).await.with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The file holds access codes and tokens.
        let _ = tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await;
    }
    tokio::fs::rename(&tmp, path).await.with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("as-store-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("state.json");
        let store = Store::open(&path).await.unwrap();
        store.update(|d| d.default_printer_id = Some("abc".into())).await.unwrap();
        drop(store);
        let store = Store::open(&path).await.unwrap();
        assert_eq!(store.read(|d| d.default_printer_id.clone()).await.as_deref(), Some("abc"));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    fn job(id: &str, created_at: i64) -> Job {
        serde_json::from_value(serde_json::json!({
            "id": id, "created_at": created_at, "updated_at": created_at, "state": "finished", "error": null, "log": [],
            "source": "upload", "model_url": "", "model_id": "", "model_title": id, "model_cover": null,
            "file": {"id": "upload", "name": "a.3mf", "kind": "3mf", "size_bytes": null, "thumbnail": null, "printer": null,
                     "compatible": [], "plates": null, "need_ams": null, "filaments": []},
            "inspection": null, "slice": null, "print": null
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn moves_jobs_from_state_json_to_the_database() {
        let dir = std::env::temp_dir().join(format!("as-store-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("state.json");
        // A pre-alpha state file, jobs newest first.
        let old = serde_json::json!({"version": 1, "jobs": [job("b", 20), job("a", 10)], "default_printer_id": "p"});
        tokio::fs::write(&path, serde_json::to_vec(&old).unwrap()).await.unwrap();
        let store = Store::open(&path).await.unwrap();
        let state: serde_json::Value = serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert!(state.get("jobs").is_none(), "state.json no longer holds jobs");
        assert_eq!(state["default_printer_id"], "p");
        store.update(|d| {
            d.jobs.insert(0, job("c", 30));
            d.jobs.retain(|j| j.id != "a");
            d.jobs[1].model_title = "renamed".into();
        })
        .await
        .unwrap();
        drop(store);
        let store = Store::open(&path).await.unwrap();
        let jobs = store.read(|d| d.jobs.iter().map(|j| (j.id.clone(), j.model_title.clone())).collect::<Vec<_>>()).await;
        assert_eq!(jobs, vec![("c".into(), "c".into()), ("b".into(), "renamed".into())]);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
