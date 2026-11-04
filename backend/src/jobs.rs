//! The print pipeline: link -> download -> inspect -> slice -> upload -> start.

use crate::{
    models::{
        PrintOutcome,
        ExtraChoice, FilamentInfo, Inspection, Job, JobState, MeshTransform, ModelFile, PlateExtra, PlateInfo, PrintOptions, PrintStart, Printer, QueueItem,
        ResolvedModel, SliceOptions, SliceOutcome, Source, now,
    },
    printer::{ftp, mapping},
    slicer::{SliceExtra, SliceFilament, SliceRequest},
    sources::{self, makerworld, printables},
    state::Shared,
    threemf,
};
use anyhow::{Context, anyhow, bail};
use std::path::PathBuf;

// ------------------------------------------------------------------ helpers

pub async fn resolve(state: &Shared, input: &str) -> anyhow::Result<ResolvedModel> {
    let link = sources::parse_link(input)?;
    let cache_key = format!("{}#{}", link.url, link.profile_id.clone().unwrap_or_default());
    if let Some(m) = state.cached_model(&cache_key).await {
        return Ok(m);
    }
    let model = match link.source {
        Source::Makerworld => {
            let token = state.store.read(|d| d.bambu.token.clone()).await;
            makerworld::resolve(&state.http, &link, token.as_deref()).await?
        }
        Source::Printables => printables::resolve(&state.http, &link).await?,
        Source::Thingiverse => {
            let token = thingiverse_token(state).await?;
            sources::thingiverse::resolve(&state.http, &link, &token).await?
        }
        Source::Upload | Source::External => bail!("not a link"),
    };
    state.cache_model(&cache_key, &model).await;
    Ok(model)
}

async fn thingiverse_token(state: &Shared) -> anyhow::Result<String> {
    state
        .store
        .read(|d| d.thingiverse_token.clone())
        .await
        .ok_or_else(|| anyhow!("Thingiverse needs a developer token. Add one under Settings → Model sites."))
}

pub fn job_dir(state: &Shared, id: &str) -> PathBuf {
    state.cfg.jobs_dir().join(id)
}

pub async fn get_job(state: &Shared, id: &str) -> Option<Job> {
    state.store.read(|d| d.jobs.iter().find(|j| j.id == id).cloned()).await
}

/// Apply `f` to a job and persist. Returns the updated job.
pub async fn update_job(state: &Shared, id: &str, f: impl FnOnce(&mut Job)) -> anyhow::Result<Job> {
    state
        .store
        .update(|d| {
            let job = d.jobs.iter_mut().find(|j| j.id == id)?;
            f(job);
            job.updated_at = now();
            Some(job.clone())
        })
        .await?
        .ok_or_else(|| anyhow!("job not found"))
}

async fn log(state: &Shared, id: &str, line: impl Into<String>) {
    let line = line.into();
    let _ = update_job(state, id, |j| j.push_log(line)).await;
}

async fn fail(state: &Shared, id: &str, err: &anyhow::Error) {
    let msg = format!("{err:#}");
    if let Ok(job) = update_job(state, id, |j| j.fail(msg)).await {
        crate::events::problem(state, &job).await;
    }
}

pub async fn find_printer(state: &Shared, id: Option<&str>) -> anyhow::Result<Printer> {
    state
        .store
        .read(|d| {
            let wanted = id.map(str::to_string).or_else(|| d.default_printer_id.clone());
            match wanted {
                Some(w) => d.printers.iter().find(|p| p.id == w).cloned(),
                None if d.printers.len() == 1 => d.printers.first().cloned(),
                None => None,
            }
        })
        .await
        .ok_or_else(|| {
            if id.is_some() { anyhow!("printer not found") } else { anyhow!("choose a printer (no default printer is set)") }
        })
}

// ------------------------------------------------------------------ storage

const FILES_REMOVED: &str = "The files were removed to save space. Use “Print again” to download it again.";

/// Remove the files of jobs not touched for `keep.keep_files_days` days,
/// and of all but the newest `keep.keep_files_count` jobs that have files
/// (either rule, 0 = off). Favourites, queued jobs and anything in progress
/// are kept and don't count. Returns (jobs, bytes).
pub async fn clean_files(state: &Shared, keep: crate::models::StorageSettings) -> anyhow::Result<(usize, u64)> {
    if keep.keep_files_days == 0 && keep.keep_files_count == 0 {
        return Ok((0, 0));
    }
    let cutoff = now() - keep.keep_files_days as i64 * 86_400;
    let old: Vec<String> = state
        .store
        .read(|d| {
            let mut candidates: Vec<&Job> = d
                .jobs
                .iter()
                .filter(|j| {
                    !j.favorite
                        && !j.files_removed
                        && !j.state.is_busy()
                        && !matches!(j.state, JobState::Queued | JobState::Started)
                        && !d.queue.iter().any(|q| q.job_id == j.id)
                })
                .collect();
            candidates.sort_by_key(|j| std::cmp::Reverse(j.updated_at));
            candidates
                .iter()
                .enumerate()
                .filter(|(i, j)| {
                    (keep.keep_files_days > 0 && j.updated_at < cutoff) || (keep.keep_files_count > 0 && *i >= keep.keep_files_count as usize)
                })
                .map(|(_, j)| j.id.clone())
                .collect()
        })
        .await;
    let mut freed = 0;
    for id in &old {
        let dir = job_dir(state, id);
        let d = dir.clone();
        freed += tokio::task::spawn_blocking(move || dir_size(&d)).await.unwrap_or(0);
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let _ = update_job(state, id, |j| {
            j.files_removed = true;
            j.push_log("Files removed to save space");
        })
        .await;
    }
    if !old.is_empty() {
        tracing::info!("removed the files of {} old jobs ({} MB)", old.len(), freed / 1_000_000);
    }
    Ok((old.len(), freed))
}

pub fn dir_size(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// MakerWorld pictures kept for the browser (`api::jobs::makerworld_image`).
pub fn image_cache_dir(state: &Shared) -> PathBuf {
    state.cfg.data_dir.join("cache").join("makerworld")
}

/// At most this much, and nothing unseen for this long.
const IMAGE_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const IMAGE_CACHE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);

/// Trim a picture cache: drop files unused for `max_age`, then the least
/// recently used until it's under `max_bytes`. A file's modified time is its
/// last use (a cache hit sets it). Returns (files, bytes) removed.
pub fn trim_cache(dir: &std::path::Path, max_bytes: u64, max_age: std::time::Duration) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return (0, 0) };
    let now = std::time::SystemTime::now();
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(now), m.len(), e.path()))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0)); // newest first
    let (mut kept, mut removed, mut freed) = (0u64, 0, 0);
    for (used, size, path) in files {
        let old = now.duration_since(used).unwrap_or_default() > max_age;
        if old || kept + size > max_bytes {
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
                freed += size;
            }
        } else {
            kept += size;
        }
    }
    (removed, freed)
}

/// Clean up old job files and the picture cache once an hour.
pub async fn clean_files_hourly(state: Shared) {
    loop {
        let keep = state.store.read(|d| d.storage).await;
        if let Err(e) = clean_files(&state, keep).await {
            tracing::warn!("cleaning up job files: {e:#}");
        }
        let dir = image_cache_dir(&state);
        if let Ok((n, bytes)) = tokio::task::spawn_blocking(move || trim_cache(&dir, IMAGE_CACHE_MAX_BYTES, IMAGE_CACHE_MAX_AGE)).await
            && n > 0
        {
            tracing::info!("removed {n} cached pictures ({} MB)", bytes / 1_000_000);
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

/// Called on startup: jobs that were mid-way are marked as interrupted;
/// prints waiting for an offline printer keep waiting.
pub async fn recover(state: &Shared) -> anyhow::Result<()> {
    let missing = missing_files(state).await;
    let waiting = state
        .store
        .update(|d| {
            for j in d.jobs.iter_mut().filter(|j| j.state.is_busy() && j.state != JobState::Waiting) {
                j.fail("interrupted by a restart — try again");
            }
            // Covers saved by pre-alpha builds point at MakerWorld originals browsers won't show.
            for j in d.jobs.iter_mut() {
                if let Some(c) = j.model_cover.take() {
                    j.model_cover = Some(crate::sources::makerworld::thumb(c));
                }
                if let Some(t) = j.file.thumbnail.take() {
                    j.file.thumbnail = Some(crate::sources::makerworld::thumb(t));
                }
            }
            // Lost with a restored backup or a moved data folder: say so, so
            // "Print again" downloads the model instead of copying nothing.
            for j in d.jobs.iter_mut().filter(|j| missing.contains(&j.id)) {
                j.files_removed = true;
                j.push_log("The files are gone from the data folder");
            }
            d.jobs.iter().filter(|j| j.state == JobState::Waiting).map(|j| j.id.clone()).collect::<Vec<_>>()
        })
        .await?;
    for id in waiting {
        tokio::spawn(wait_and_send(state.clone(), id));
    }
    Ok(())
}

/// Jobs that should have an input file on disk but don't.
async fn missing_files(state: &Shared) -> Vec<String> {
    let jobs: Vec<(String, std::path::PathBuf)> = state
        .store
        .read(|d| {
            d.jobs
                .iter()
                .filter(|j| !j.files_removed && !j.state.is_busy())
                .filter_map(|j| j.input_file.as_ref().map(|f| (j.id.clone(), job_dir(state, &j.id).join(f))))
                .collect()
        })
        .await;
    tokio::task::spawn_blocking(move || jobs.into_iter().filter(|(_, f)| !f.exists()).map(|(id, _)| id).collect())
        .await
        .unwrap_or_default()
}

// ------------------------------------------------------------------ create

pub async fn create(
    state: &Shared,
    url: &str,
    file_id: Option<&str>,
    auto: Option<(SliceOptions, PrintOptions)>,
) -> anyhow::Result<Job> {
    let model = resolve(state, url).await?;
    let printer_model = match &auto {
        Some((s, _)) => find_printer(state, s.printer_id.as_deref()).await.ok().map(|p| p.printer_model),
        None => None,
    };
    let file = match file_id {
        Some(id) => model.files.iter().find(|f| f.id == id).cloned().ok_or_else(|| anyhow!("file not found on the model page"))?,
        None => sources::auto_pick(&model, printer_model.as_deref())
            .cloned()
            .ok_or_else(|| anyhow!(model.notes.first().cloned().unwrap_or_else(|| "no printable files found".into())))?,
    };
    if model.source == Source::Makerworld && state.store.read(|d| d.bambu.token.is_none()).await {
        bail!("Link your Bambu account under Settings → Model sites to download from MakerWorld.");
    }

    let mut job = new_job(model.source, model.url.clone(), model.model_id.clone(), model.title.clone(), model.cover.clone(), file.clone());
    job.auto = auto.is_some();
    job.auto_slice = auto.as_ref().map(|a| a.0.clone());
    job.auto_print = auto.as_ref().map(|a| a.1.clone());
    job.push_log(format!("Downloading “{}” from {}", file.name, model.url));

    insert_job(state, &job).await?;

    let st = state.clone();
    let id = job.id.clone();
    tokio::spawn(async move {
        if let Err(e) = download(&st, &id, &model, &file).await {
            fail(&st, &id, &e).await;
            return;
        }
        let Some(job) = get_job(&st, &id).await else { return };
        if let (true, Some(opts)) = (job.auto, job.auto_slice.clone())
            && let Err(e) = slice(&st, &id, opts).await {
                fail(&st, &id, &e).await;
            }
    });
    Ok(job)
}

pub(crate) async fn insert_job(state: &Shared, job: &Job) -> anyhow::Result<()> {
    let max = state.cfg.max_jobs;
    let job = job.clone();
    let removed = state
        .store
        .update(|d| {
            d.jobs.insert(0, job);
            let mut removed = Vec::new();
            // Drop the oldest jobs, but never favourites, queued ones or
            // prints that ran: those are the print history.
            while d.jobs.iter().filter(|j| !is_print_record(j)).count() > max {
                let Some(pos) = d
                    .jobs
                    .iter()
                    .rposition(|j| !j.favorite && j.state != JobState::Queued && !j.state.in_use() && !is_print_record(j))
                else {
                    break;
                };
                removed.push(d.jobs.remove(pos).id);
            }
            removed
        })
        .await?;
    for id in removed {
        let _ = tokio::fs::remove_dir_all(job_dir(state, &id)).await;
    }
    Ok(())
}

/// A print that ended: part of the history for good, unless someone removes it.
fn is_print_record(j: &Job) -> bool {
    j.print.as_ref().is_some_and(|p| p.outcome.is_some())
}

pub(crate) fn new_job(source: Source, url: String, model_id: String, title: String, cover: Option<String>, file: ModelFile) -> Job {
    let t = now();
    Job {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        created_at: t,
        updated_at: t,
        state: JobState::Downloading,
        error: None,
        log: vec![],
        source,
        model_url: url,
        model_id,
        model_title: title,
        model_cover: cover,
        file,
        inspection: None,
        slice: None,
        print: None,
        auto: false,
        auto_slice: None,
        auto_print: None,
        input_file: None,
        output_file: None,
        extras: vec![],
        favorite: false,
        copied_from: None,
        plate_label: None,
        auto_queue: false,
        waiting_until: None,
        pending_print: None,
        cost: None,
        files_removed: false,
        photo: false,
    }
}

/// Input file name inside the job directory for a source file name.
fn input_name(file_name: &str, kind: &str) -> String {
    let ext = if kind == "profile" {
        "3mf".to_string()
    } else {
        match sources::extension(file_name).as_str() {
            "gcode.3mf" => "3mf".to_string(),
            "stp" => "step".to_string(),
            e if sources::SLICEABLE.contains(&e) => e.to_string(),
            _ => kind.to_string(),
        }
    };
    format!("input.{ext}")
}

/// Accept a file uploaded from the browser (any site, or your own model).
pub async fn create_upload<S, E>(state: &Shared, file_name: &str, chunks: S) -> anyhow::Result<Job>
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let (title, file) = uploaded_file(file_name)?;
    let mut job = new_job(Source::Upload, String::new(), String::new(), title, None, file.clone());
    job.push_log(format!("Receiving {file_name}"));
    let dir = job_dir(state, &job.id);
    tokio::fs::create_dir_all(&dir).await?;
    let name = input_name(file_name, &file.kind);
    let size = match save_upload(chunks, &dir.join(&name)).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
    };
    insert_job(state, &job).await?;
    log(state, &job.id, format!("Received {:.1} MB", size as f64 / 1_048_576.0)).await;
    if let Err(e) = finish_input(state, &job.id, name).await {
        fail(state, &job.id, &e).await;
    }
    get_job(state, &job.id).await.ok_or_else(|| anyhow!("job vanished"))
}

/// The title and file entry of an upload, if we can slice that kind of file.
fn uploaded_file(file_name: &str) -> anyhow::Result<(String, ModelFile)> {
    let ext = sources::extension(file_name);
    if !sources::SLICEABLE.contains(&ext.as_str()) && ext != "gcode.3mf" {
        bail!("upload a 3MF, STL, OBJ or STEP file");
    }
    let title = std::path::Path::new(file_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.to_string());
    let title = title.trim_end_matches(".gcode.3mf").trim_end_matches(&format!(".{ext}")).to_string();
    Ok((title, ModelFile {
        id: "upload".into(),
        name: file_name.to_string(),
        kind: if ext == "gcode.3mf" { "3mf".into() } else { ext.clone() },
        size_bytes: None,
        thumbnail: None,
        printer: None,
        compatible: vec![],
        plates: None,
        need_ams: None,
        filaments: vec![],
        download_type: None,
        folder: None,
        prediction_secs: None,
        weight_g: None,
    }))
}

/// Write an uploaded stream to disk with the size cap. Returns the bytes written.
async fn save_upload<S, E>(mut chunks: S, dest: &std::path::Path) -> anyhow::Result<u64>
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut out = tokio::fs::File::create(dest).await?;
    let mut size = 0u64;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| anyhow!("upload interrupted: {e}"))?;
        size += chunk.len() as u64;
        if size > sources::MAX_DOWNLOAD {
            bail!("file is too large");
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(size)
}

async fn download(state: &Shared, id: &str, model: &ResolvedModel, file: &ModelFile) -> anyhow::Result<()> {
    let dir = job_dir(state, id);
    tokio::fs::create_dir_all(&dir).await?;
    let name = input_name(&file.name, &file.kind);
    let dest = dir.join(&name);
    let size = fetch_file(state, model, file, &dest).await?;
    log(state, id, format!("Downloaded {:.1} MB", size as f64 / 1_048_576.0)).await;
    finish_input(state, id, name).await
}

/// Download a model's file from its site. Returns the bytes written.
async fn fetch_file(state: &Shared, model: &ResolvedModel, file: &ModelFile, dest: &std::path::Path) -> anyhow::Result<u64> {
    Ok(match model.source {
        Source::Makerworld => {
            let token = state.store.read(|d| d.bambu.token.clone()).await.ok_or_else(|| anyhow!("Bambu account not linked"))?;
            let internal = model.internal_id.as_deref().ok_or_else(|| anyhow!("MakerWorld did not return the model id"))?;
            let china = model.url.contains(".cn/");
            makerworld::download(&state.http, china, &model.model_id, internal, &file.id, &token, dest).await?.0
        }
        Source::Printables => {
            let url = printables::download_url(&state.http, &model.model_id, file).await?;
            sources::download_to(&state.http, &url, dest).await?
        }
        Source::Thingiverse => {
            let token = thingiverse_token(state).await?;
            sources::thingiverse::download(&state.http, &token, &file.id, dest).await?
        }
        Source::Upload | Source::External => bail!("uploads are not downloaded"),
    })
}

// ------------------------------------------------------------------ models added to the plate

const MAX_EXTRAS: usize = 10;
const MAX_EXTRA_COPIES: u32 = 50;

/// The job, if models can be added to or removed from its plate now.
async fn job_for_extras(state: &Shared, id: &str) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    if job.files_removed {
        bail!("{FILES_REMOVED}");
    }
    if job.extras.len() >= MAX_EXTRAS {
        bail!("A plate can take {MAX_EXTRAS} added models. Remove one to add another.");
    }
    Ok(job)
}

/// Look at a file now in the job's `extras/` folder and add it to the job.
async fn register_extra(state: &Shared, id: &str, mut extra: PlateExtra) -> anyhow::Result<Job> {
    let path = job_dir(state, id).join("extras").join(&extra.input_file);
    let looked_at = path.clone();
    let inspection = match tokio::task::spawn_blocking(move || threemf::inspect(&looked_at)).await? {
        Ok(i) => i,
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(e.context("That file can't be read"));
        }
    };
    extra.kind = inspection.kind.clone();
    extra.multicolor = inspection.filaments.len() > 1;
    extra.plates = inspection.plates;
    update_job(state, id, |j| {
        j.push_log(format!("Added “{}” to the plate", extra.title));
        j.extras.push(extra);
    })
    .await
}

fn new_extra(source: Source, url: String, title: String, cover: Option<String>, file: &ModelFile) -> PlateExtra {
    let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let input_file = input_name(&file.name, &file.kind).replacen("input", &id, 1);
    PlateExtra { id, title, source, model_url: url, cover, input_file, kind: String::new(), plates: vec![], multicolor: false }
}

/// Add a model from a link to the job's plate. Done when this returns.
pub async fn add_extra(state: &Shared, id: &str, url: &str, file_id: Option<&str>) -> anyhow::Result<Job> {
    job_for_extras(state, id).await?;
    let model = resolve(state, url).await?;
    let file = match file_id {
        Some(f) => model.files.iter().find(|x| x.id == f),
        None => sources::auto_pick(&model, None),
    }
    .cloned()
    .ok_or_else(|| anyhow!(model.notes.first().cloned().unwrap_or_else(|| "no printable files found".into())))?;
    if model.source == Source::Makerworld && state.store.read(|d| d.bambu.token.is_none()).await {
        bail!("Link your Bambu account under Settings → Model sites to download from MakerWorld.");
    }
    let extra = new_extra(model.source, model.url.clone(), model.title.clone(), model.cover.clone(), &file);
    let dir = job_dir(state, id).join("extras");
    tokio::fs::create_dir_all(&dir).await?;
    fetch_file(state, &model, &file, &dir.join(&extra.input_file)).await?;
    register_extra(state, id, extra).await
}

/// Add a file uploaded from the browser to the job's plate.
pub async fn add_extra_upload<S, E>(state: &Shared, id: &str, file_name: &str, chunks: S) -> anyhow::Result<Job>
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    job_for_extras(state, id).await?;
    let (title, file) = uploaded_file(file_name)?;
    let extra = new_extra(Source::Upload, String::new(), title, None, &file);
    let dir = job_dir(state, id).join("extras");
    tokio::fs::create_dir_all(&dir).await?;
    let dest = dir.join(&extra.input_file);
    if let Err(e) = save_upload(chunks, &dest).await {
        let _ = tokio::fs::remove_file(&dest).await;
        return Err(e);
    }
    register_extra(state, id, extra).await
}

/// Add another job's model to this job's plate: its file when still there,
/// else fetched again from its site.
pub async fn add_extra_from_job(state: &Shared, id: &str, from: &str) -> anyhow::Result<Job> {
    job_for_extras(state, id).await?;
    let old = get_job(state, from).await.ok_or_else(|| anyhow!("that job is gone"))?;
    if old.source == Source::External {
        bail!("This print wasn't started from AirSlice, so AirSlice has no file for it.");
    }
    let path = old.input_file.as_ref().filter(|_| !old.files_removed).map(|f| job_dir(state, from).join(f));
    let file_there = match &path {
        Some(p) => tokio::fs::try_exists(p).await.unwrap_or(false),
        None => false,
    };
    if !file_there {
        if old.source == Source::Upload {
            bail!("The file of “{}” is gone. Upload it again to add it.", old.model_title);
        }
        return add_extra(state, id, &old.model_url, Some(&old.file.id)).await;
    }
    let extra = new_extra(old.source, old.model_url.clone(), old.model_title.clone(), old.model_cover.clone(), &old.file);
    let dir = job_dir(state, id).join("extras");
    tokio::fs::create_dir_all(&dir).await?;
    if let Some(p) = path {
        tokio::fs::copy(p, dir.join(&extra.input_file)).await.context("copying the model")?;
    }
    register_extra(state, id, extra).await
}

pub async fn remove_extra(state: &Shared, id: &str, extra_id: &str) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    let extra = job.extras.iter().find(|e| e.id == extra_id).ok_or_else(|| anyhow!("that model isn't on this plate"))?;
    let _ = tokio::fs::remove_file(job_dir(state, id).join("extras").join(&extra.input_file)).await;
    update_job(state, id, |j| j.extras.retain(|e| e.id != extra_id)).await
}

/// Where each added model's filament comes from: the base's slot when a tray
/// is already in use, else a new slot after the base's. No tray = the first.
/// Returns one slot per model and the trays of the new slots.
fn extra_slots(base_trays: &[i32], base_slots: usize, trays: &[Option<i32>]) -> (Vec<u32>, Vec<i32>) {
    let mut new: Vec<i32> = Vec::new();
    let slots = trays
        .iter()
        .map(|tray| match tray.filter(|t| *t >= 0) {
            None => 1,
            Some(t) => {
                if let Some(i) = base_trays.iter().position(|b| *b == t) {
                    return i as u32 + 1;
                }
                let at = new.iter().position(|n| *n == t).unwrap_or_else(|| {
                    new.push(t);
                    new.len() - 1
                });
                (base_slots + at + 1) as u32
            }
        })
        .collect();
    (slots, new)
}

/// The added models a slice asks for, checked.
fn chosen_extras(job: &Job, opts: &SliceOptions) -> anyhow::Result<Vec<(PlateExtra, ExtraChoice)>> {
    let mut out = Vec::new();
    for choice in &opts.extras {
        let extra = job.extras.iter().find(|e| e.id == choice.id).ok_or_else(|| anyhow!("an added model is no longer on this plate"))?;
        if !(1..=MAX_EXTRA_COPIES).contains(&choice.copies.unwrap_or(1)) {
            bail!("copies must be between 1 and {MAX_EXTRA_COPIES}");
        }
        if let Some(p) = choice.plate
            && !extra.plates.iter().any(|x| x.index == p) {
                bail!("“{}” has no plate {p}", extra.title);
            }
        out.push((extra.clone(), choice.clone()));
    }
    Ok(out)
}

/// Inspect the input file and mark the job ready.
async fn finish_input(state: &Shared, id: &str, name: String) -> anyhow::Result<()> {
    let path = job_dir(state, id).join(&name);
    let inspection: Inspection = tokio::task::spawn_blocking(move || threemf::inspect(&path)).await??;
    let summary = match inspection.kind.as_str() {
        "mesh" => "Single mesh — will be auto-arranged on the plate".to_string(),
        "sliced" => format!("Already sliced, {} plate(s)", inspection.plates.len()),
        _ => format!(
            "{} plate(s){}",
            inspection.plates.len(),
            inspection.printer_model.as_ref().map(|m| format!(", set up for {m}")).unwrap_or_default()
        ),
    };
    update_job(state, id, |j| {
        j.input_file = Some(name);
        j.inspection = Some(inspection);
        j.state = JobState::Ready;
        j.push_log(summary);
    })
    .await?;
    Ok(())
}

// ------------------------------------------------------------------ slice

/// What settings a slice will use, decided before running.
#[derive(Debug, PartialEq)]
pub enum Plan {
    Presliced,
    /// The project's own settings; a pre-sliced file is sliced again when
    /// copies are asked for.
    Project { copies: u32 },
    Printer { process: Option<String>, filament: Option<String>, arrange: bool, transform: MeshTransform },
}

/// The objects to keep when only some of the plate's are chosen; `None`
/// when everything prints (or the plate's objects aren't known).
pub fn chosen_objects(inspection: &Inspection, opts: &SliceOptions) -> anyhow::Result<Option<Vec<String>>> {
    let Some(ids) = &opts.objects else { return Ok(None) };
    let plate = opts.plate.or_else(|| inspection.plates.first().map(|p| p.index)).unwrap_or(1);
    let on_plate = inspection.plates.iter().find(|p| p.index == plate).map(|p| p.objects.as_slice()).unwrap_or(&[]);
    if on_plate.is_empty() {
        return Ok(None);
    }
    let keep: Vec<String> = on_plate.iter().filter(|o| ids.contains(&o.id)).map(|o| o.id.clone()).collect();
    if keep.is_empty() {
        bail!("choose at least one object to print");
    }
    Ok((keep.len() < on_plate.len()).then_some(keep))
}

pub fn plan(inspection: &Inspection, printer: &Printer, opts: &SliceOptions) -> Plan {
    let copies = opts.transform.copies();
    if inspection.kind == "sliced" {
        // Copies, leaving objects out or adding models means slicing the file again.
        let partial = chosen_objects(inspection, opts).ok().flatten().is_some();
        return if copies > 1 || partial || !opts.extras.is_empty() { Plan::Project { copies } } else { Plan::Presliced };
    }
    let process = opts.process_profile.clone().or_else(|| printer.process_profile.clone());
    if inspection.kind == "mesh" {
        let filament = opts.filament_profile.clone().or_else(|| printer.filament_profile.clone());
        return Plan::Printer { process, filament, arrange: true, transform: opts.transform.clone() };
    }
    let same_printer = inspection
        .printer_model
        .as_deref()
        .map(|m| m.eq_ignore_ascii_case(&printer.printer_model))
        .unwrap_or(false);
    let transform = MeshTransform { copies: opts.transform.copies, ..Default::default() };
    match opts.settings.as_deref() {
        Some("project") => Plan::Project { copies },
        Some("printer") => Plan::Printer { process, filament: None, arrange: false, transform },
        _ if same_printer => Plan::Project { copies },
        // Projects without printer info (e.g. from other slicers) or made
        // for another printer get this printer's profiles.
        _ => Plan::Printer { process, filament: None, arrange: false, transform },
    }
}

/// The trays chosen before slicing, as filament profiles to slice with (one
/// per project filament, in the tray's colour) and the trays to pre-select
/// for printing. Filaments without a tray keep their own material and colour.
struct TrayChoice {
    filaments: Vec<SliceFilament>,
    preferred: Vec<i32>,
    note: String,
}

async fn tray_choice(state: &Shared, printer: &Printer, inspection: &Inspection, opts: &SliceOptions, extra_trays: &[i32]) -> anyhow::Result<Option<TrayChoice>> {
    let ids = opts.tray_ids.clone().or_else(|| opts.tray_id.map(|t| vec![t])).unwrap_or_default();
    if !ids.iter().any(|t| *t >= 0) && extra_trays.is_empty() {
        return Ok(None);
    }
    let mesh = inspection.kind == "mesh";
    let slots: Vec<FilamentInfo> = if mesh {
        vec![FilamentInfo { kind: String::new(), color: String::new() }]
    } else {
        inspection.filaments.clone()
    };
    if ids.len() > slots.len().max(1) {
        bail!("this file has {} filament(s), but {} trays were chosen", slots.len(), ids.len());
    }
    let status = state.printers.status(&printer.id).await;
    let usable = status.usable_trays();
    let index = state.slicer.profiles().await?;
    let machine = &printer.machine_profile;
    let mut choice = TrayChoice { filaments: Vec::new(), preferred: Vec::new(), note: String::new() };
    let mut notes = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        match ids.get(i).copied().filter(|t| *t >= 0) {
            Some(id) => {
                let tray = usable
                    .iter()
                    .find(|t| t.id == id)
                    .ok_or_else(|| anyhow!("a chosen tray is empty or the printer isn't reporting it right now"))?;
                let profile = opts
                    .filament_profile
                    .clone()
                    .filter(|_| mesh)
                    .or_else(|| index.filament_for_tray(machine, tray.info_idx.as_deref(), &tray.kind))
                    .ok_or_else(|| anyhow!("no {} filament profile for this printer ({})", tray.kind, tray.label()))?;
                notes.push(format!("{profile} from {}", tray.label()));
                choice.filaments.push(SliceFilament { profile, color: Some(tray.color.clone()) });
                choice.preferred.push(id);
            }
            None => {
                let profile = index
                    .filament_for_tray(machine, None, &slot.kind)
                    .or_else(|| index.default_filament(machine))
                    .ok_or_else(|| anyhow!("no filament profile for this printer"))?;
                let color = Some(slot.color.clone()).filter(|c| !c.is_empty());
                choice.filaments.push(SliceFilament { profile, color });
                choice.preferred.push(-1);
            }
        }
    }
    // Filaments of added models come after the file's own.
    for id in extra_trays {
        let tray = usable.iter().find(|t| t.id == *id).ok_or_else(|| anyhow!("a chosen tray is empty or the printer isn't reporting it right now"))?;
        let profile = index
            .filament_for_tray(machine, tray.info_idx.as_deref(), &tray.kind)
            .ok_or_else(|| anyhow!("no {} filament profile for this printer ({})", tray.kind, tray.label()))?;
        notes.push(format!("{profile} from {}", tray.label()));
        choice.filaments.push(SliceFilament { profile, color: Some(tray.color.clone()) });
        choice.preferred.push(*id);
    }
    choice.note = if notes.is_empty() { String::new() } else { format!(" using {}", notes.join(", ")) };
    Ok(Some(choice))
}

pub async fn slice(state: &Shared, id: &str, opts: SliceOptions) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    if job.files_removed {
        bail!("{FILES_REMOVED}");
    }
    let inspection = job.inspection.clone().ok_or_else(|| anyhow!("file is not downloaded yet"))?;
    let input = job_dir(state, id).join(job.input_file.as_deref().ok_or_else(|| anyhow!("file is missing"))?);
    let printer = find_printer(state, opts.printer_id.as_deref()).await?;
    let plate = match opts.plate {
        Some(p) => {
            if !inspection.plates.iter().any(|x| x.index == p) {
                bail!("plate {p} does not exist");
            }
            p
        }
        None => inspection.plates.first().map(|p| p.index).unwrap_or(1),
    };
    if !opts.transform.is_identity() {
        if inspection.kind != "mesh" && opts.transform.changes_shape() {
            bail!("scale and rotation are only available for STL, OBJ and STEP files");
        }
        opts.transform.validate().map_err(|e| anyhow!(e))?;
    }
    opts.tweaks.validate().map_err(|e| anyhow!(e))?;
    let tweaks = opts.tweaks.clone();
    let bed_type = match opts.bed_type.as_deref().or(printer.default_plate.as_deref()) {
        Some(p) => Some(crate::slicer::build_plate(p).ok_or_else(|| anyhow!("unknown build plate {p:?}"))?.value.to_string()),
        None => None,
    };
    let keep = chosen_objects(&inspection, &SliceOptions { plate: Some(plate), ..opts.clone() })?;
    let chosen = chosen_extras(&job, &opts)?;
    if !chosen.is_empty() && inspection.kind != "mesh" && opts.transform.copies() > 1 {
        bail!("A 3MF project can't have copies together with added models. Choose 1 copy, or add a copy of the file as another model.");
    }
    // A project without filament information has no slots to add to.
    let mesh = inspection.kind == "mesh";
    let base_slots = if mesh { 1 } else { inspection.filaments.len() };
    let base_trays: Vec<i32> = opts.tray_ids.clone().or_else(|| opts.tray_id.map(|t| vec![t])).unwrap_or_default();
    let wanted_trays: Vec<Option<i32>> = chosen.iter().map(|(_, c)| c.tray_id.filter(|_| base_slots > 0)).collect();
    let (slots, new_trays) = extra_slots(&base_trays, base_slots, &wanted_trays);
    let trays = tray_choice(state, &printer, &inspection, &opts, &new_trays).await?;
    let plan = plan(&inspection, &printer, &opts);
    let (filaments, preferred_trays, tray_note) = match (trays, &plan) {
        // Pre-sliced plates print as sliced; the trays only pre-select the mapping.
        (Some(t), Plan::Presliced) => (Vec::new(), Some(t.preferred), String::new()),
        (Some(t), _) => (t.filaments, Some(t.preferred), t.note),
        (None, _) => (Vec::new(), None, String::new()),
    };
    let filament_used = match &plan {
        Plan::Printer { filament, arrange: true, .. } => filaments.first().map(|f| f.profile.clone()).or(filament.clone()),
        _ => None,
    };
    let plate_note = match (&plan, &bed_type) {
        (Plan::Presliced, _) | (_, None) => String::new(),
        (_, Some(b)) => format!(" on the {}", crate::slicer::build_plate(b).map(|p| p.label).unwrap_or(b)),
    };
    let objects_note = match (&keep, inspection.plates.iter().find(|p| p.index == plate)) {
        (Some(k), Some(p)) => format!(" ({} of {} objects)", k.len(), p.objects.len()),
        _ => String::new(),
    };
    let tf_note = match &plan {
        Plan::Printer { transform, .. } if !transform.describe().is_empty() => format!(" ({})", transform.describe()),
        Plan::Project { copies } if *copies > 1 => format!(" ({copies} copies)"),
        _ => String::new(),
    };
    let tweak_note = match &plan {
        Plan::Presliced => String::new(),
        _ if tweaks.is_empty() => String::new(),
        _ => format!(", changed: {}", tweaks.describe()),
    };
    let extras_note = if chosen.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = chosen
            .iter()
            .map(|(e, c)| match c.copies.unwrap_or(1) {
                1 => format!("“{}”", e.title),
                n => format!("{n} × “{}”", e.title),
            })
            .collect();
        format!(", with {}", names.join(", "))
    };
    let extras_dir = job_dir(state, id).join("extras");
    let slice_extras: Vec<SliceExtra> = chosen
        .iter()
        .zip(&slots)
        .map(|((e, c), slot)| SliceExtra {
            path: extras_dir.join(&e.input_file),
            plate: c.plate.or_else(|| e.plates.first().map(|p| p.index)).unwrap_or(1),
            copies: c.copies.unwrap_or(1),
            slot: *slot,
            orient: e.kind == "mesh",
        })
        .collect();

    remove_from_queue(state, id).await?;
    let job = update_job(state, id, |j| {
        j.state = JobState::Slicing;
        j.error = None;
        j.slice = None;
        j.push_log(match &plan {
            Plan::Presliced => format!("Using the pre-sliced plate {plate}"),
            Plan::Project { .. } => {
                format!("Slicing plate {plate}{objects_note} for {}{plate_note} with the project's own settings{tf_note}{tray_note}{tweak_note}{extras_note}", printer.name)
            }
            Plan::Printer { process, .. } => format!(
                "Slicing plate {plate}{objects_note} for {}{plate_note} with {}{}{tf_note}{tray_note}{tweak_note}{extras_note}",
                printer.name,
                printer.machine_profile,
                process.as_ref().map(|p| format!(" / {p}")).unwrap_or_default()
            ),
        });
    })
    .await?;

    let expected_objects = match &keep {
        Some(k) => k.len() as u32,
        None => inspection.plates.iter().find(|p| p.index == plate).map(|p| p.object_count).unwrap_or(0),
    };
    let st = state.clone();
    let id = id.to_string();
    let asked = SliceOptions { printer_id: Some(printer.id.clone()), plate: Some(plate), ..opts };
    tokio::spawn(async move {
        if let Err(e) = run_slice(&st, &id, &input, &printer, plate, plan, &filaments, bed_type.as_deref(), keep, expected_objects, tweaks, &slice_extras).await {
            fail(&st, &id, &e).await;
            let _ = update_job(&st, &id, |j| j.auto_queue = false).await;
            return;
        }
        let _ = update_job(&st, &id, |j| {
            if let Some(s) = j.slice.as_mut() {
                s.options = Some(asked);
                if preferred_trays.is_some() || filament_used.is_some() {
                    s.preferred_trays = preferred_trays.clone();
                    if s.settings == "printer" {
                        s.filament_profile = filament_used.clone();
                    }
                }
            }
        })
        .await;
        let Some(job) = get_job(&st, &id).await else { return };
        crate::events::sliced(&st, &job).await;
        if job.auto_queue {
            match enqueue(&st, &id, PrintOptions::default()).await {
                // Done: slicing it again by hand shows it, not queue it.
                Ok(_) => {
                    let _ = update_job(&st, &id, |j| j.auto_queue = false).await;
                }
                Err(e) => fail(&st, &id, &e).await,
            }
            return;
        }
        if job.auto {
            let opts = job.auto_print.clone().unwrap_or_default();
            if let Err(e) = print(&st, &id, opts).await {
                fail(&st, &id, &e).await;
            }
        }
    });
    Ok(job)
}

/// OrcaSlicer moves copies that don't fit to another plate without saying
/// so; count what landed on ours.
async fn check_copies(sliced: &std::path::Path, plate: u32, per_copy: u32, copies: u32) -> anyhow::Result<()> {
    let path = sliced.to_path_buf();
    let (plates, _) = tokio::task::spawn_blocking(move || crate::threemf::slice_results(&path)).await??;
    let on_plate = plates.iter().find(|p| p.index == plate).map(|p| p.object_count).unwrap_or(0);
    // Without object info in the result there is nothing to compare.
    if on_plate == 0 {
        return Ok(());
    }
    let fit = fitted_copies(on_plate, per_copy);
    if fit < copies {
        if fit <= 1 {
            bail!("{copies} copies don't fit on the plate, only one does. Print a single copy, or queue it {copies} times.");
        }
        bail!("Only {fit} of {copies} copies fit on the plate. Choose {fit} or fewer and slice again.");
    }
    Ok(())
}

/// Added models that don't fit go to another plate without a word, like copies.
async fn check_all_placed(sliced: &std::path::Path, plate: u32, expected: u32) -> anyhow::Result<()> {
    let path = sliced.to_path_buf();
    let (plates, _) = tokio::task::spawn_blocking(move || crate::threemf::slice_results(&path)).await??;
    let on_plate = plates.iter().find(|p| p.index == plate).map(|p| p.object_count).unwrap_or(0);
    if on_plate != 0 && on_plate < expected {
        bail!("Only {on_plate} of {expected} objects fit on the plate. Remove a model or lower the copies, and slice again.");
    }
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn fitted_copies(on_plate: u32, per_copy: u32) -> u32 {
    on_plate / per_copy.max(1)
}

#[allow(clippy::too_many_arguments)]
async fn run_slice(
    state: &Shared,
    id: &str,
    input: &PathBuf,
    printer: &Printer,
    plate: u32,
    plan: Plan,
    filaments: &[SliceFilament],
    bed_type: Option<&str>,
    keep: Option<Vec<String>>,
    expected_objects: u32,
    tweaks: crate::models::SliceTweaks,
    extras: &[SliceExtra],
) -> anyhow::Result<()> {
    let mut plate = plate;
    let overrides = if matches!(plan, Plan::Presliced) { Vec::new() } else { tweaks.settings() };
    let dir = job_dir(state, id);
    // Only some objects: take the others off the plate first.
    let edited;
    let input = match keep {
        Some(keep) if !matches!(plan, Plan::Presliced) => {
            let (src, dest) = (input.clone(), dir.join("input-objects.3mf"));
            edited = dest.clone();
            tokio::task::spawn_blocking(move || threemf::keep_plate_objects(&src, &dest, plate, &keep)).await??;
            &edited
        }
        _ => input,
    };
    let (output, settings) = match &plan {
        Plan::Presliced => (input.clone(), "presliced"),
        Plan::Project { .. } | Plan::Printer { .. } => {
            let (machine, process, default_filament, mesh, label) = match &plan {
                Plan::Printer { process, filament, arrange, .. } => {
                    let index = state.slicer.profiles().await?;
                    if !index.has_machine(&printer.machine_profile) {
                        bail!("printer profile {:?} is not available in this OrcaSlicer", printer.machine_profile);
                    }
                    let choices = index.choices_for(&printer.machine_profile);
                    let process = process.clone().or(choices.default_process.clone());
                    let filament = match (*arrange, filament.clone()) {
                        (true, None) => index.default_filament(&printer.machine_profile),
                        (_, f) => f,
                    };
                    (Some(printer.machine_profile.clone()), process, filament, *arrange, "printer")
                }
                _ => (None, None, None, false, "project"),
            };
            let (transform, copies) = match &plan {
                Plan::Printer { transform, .. } => (transform.clone(), transform.copies()),
                Plan::Project { copies } => (MeshTransform::default(), *copies),
                Plan::Presliced => (MeshTransform::default(), 1),
            };
            // Meshes without a chosen tray slice with one default filament.
            let filaments: Vec<SliceFilament> = match (filaments.is_empty(), default_filament) {
                (true, Some(profile)) => vec![SliceFilament { profile, color: None }],
                _ => filaments.to_vec(),
            };
            let started = std::time::Instant::now();
            let report = |p: crate::models::SliceProgress| state.set_progress(id, Some(p));
            let out = state
                .slicer
                .slice(SliceRequest {
                    input,
                    work_dir: &dir,
                    plate,
                    machine: machine.as_deref(),
                    process: process.as_deref(),
                    filaments: &filaments,
                    bed_type,
                    plates: &printer.plates,
                    overrides: &overrides,
                    progress: Some(&report),
                    // Copies of a project are laid out again; bare meshes always are.
                    arrange: mesh || copies > 1,
                    orient: mesh && transform.orient(),
                    scale: transform.scale_factor(),
                    rotate_z: transform.rotation(),
                    copies,
                    extras,
                })
                .await;
            state.set_progress(id, None);
            let out = match out {
                Ok(o) => o,
                Err(e) => return Err(e),
            };
            for note in &out.notes {
                log(state, id, capitalize(note)).await;
            }
            if !out.adjusted.is_empty() {
                log(
                    state,
                    id,
                    format!("Used OrcaSlicer's defaults for settings it doesn't accept from this project: {}", out.adjusted.join(", ")),
                )
                .await;
            }
            for line in out.log_tail.iter().rev().take(3).rev() {
                log(state, id, format!("orca: {line}")).await;
            }
            log(state, id, format!("Sliced in {:.0} s", started.elapsed().as_secs_f64())).await;
            let final_path = dir.join("sliced.3mf");
            if out.output != final_path {
                tokio::fs::rename(&out.output, &final_path).await.context("saving sliced file")?;
            }
            if let Some(p) = out.plate {
                plate = p;
            }
            if !extras.is_empty() {
                // A mesh base counts as one object per copy.
                let base = if mesh { copies } else { expected_objects };
                check_all_placed(&final_path, plate, base + out.extra_objects).await?;
            } else if copies > 1 {
                check_copies(&final_path, plate, expected_objects.max(1), copies).await?;
            }
            (final_path, label)
        }
    };

    let path = output.clone();
    let (plates, slots) = tokio::task::spawn_blocking(move || threemf::slice_results(&path)).await??;
    let result = plates
        .iter()
        .find(|p| p.index == plate)
        .or_else(|| (plates.len() == 1).then(|| &plates[0]))
        .ok_or_else(|| anyhow!("the sliced file has no G-code for plate {plate}"))?;
    let gcode_entry = result.gcode_entry.clone().ok_or_else(|| anyhow!("the sliced file has no G-code for plate {plate}"))?;
    let outcome = SliceOutcome {
        printer_id: printer.id.clone(),
        plate: result.index,
        gcode_entry,
        filament_slots: slots,
        filaments: result.filaments.clone(),
        prediction_secs: result.prediction_secs,
        weight_g: result.weight_g,
        settings: settings.to_string(),
        transform: match &plan {
            Plan::Printer { transform, .. } if !transform.is_identity() => Some(transform.clone()),
            _ => None,
        },
        filament_profile: None,
        preferred_trays: None,
        bed_type: threemf::project_setting(&output, "curr_bed_type"),
        tweaks: if overrides.is_empty() { Default::default() } else { tweaks },
        options: None,
        output_path: output.clone(),
    };
    let file_name = output.file_name().map(|n| n.to_string_lossy().into_owned());
    update_job(state, id, |j| {
        let summary = format!(
            "Ready: {}{}",
            outcome.prediction_secs.map(fmt_duration).unwrap_or_else(|| "time unknown".into()),
            outcome.weight_g.map(|w| format!(", {w:.1} g")).unwrap_or_default()
        );
        j.output_file = file_name;
        // Remember the result on the plate, for its card when choosing again.
        if let Some(p) = j.inspection.as_mut().filter(|_| extras.is_empty()).and_then(|i| i.plates.iter_mut().find(|p| p.index == outcome.plate)) {
            p.prediction_secs = outcome.prediction_secs.or(p.prediction_secs);
            p.weight_g = outcome.weight_g.or(p.weight_g);
        }
        j.slice = Some(outcome);
        j.state = JobState::Sliced;
        j.push_log(summary);
    })
    .await?;
    Ok(())
}

pub fn fmt_duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 { format!("{h} h {m} min") } else { format!("{} min", m.max(1)) }
}

// ------------------------------------------------------------------ print

fn busy_text(s: JobState) -> String {
    if s == JobState::Started {
        "It's printing now. Use “Print again” for another copy, or wait until it's done.".into()
    } else {
        format!("job is busy ({s:?})")
    }
}

/// Job ids with something running for them: `print` deciding how to send,
/// or a `wait_and_send` loop. One of each per job at a time.
static STARTING: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
static WAITING: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

/// Holds a job id in one of those sets until dropped.
struct Claim(&'static std::sync::Mutex<Option<std::collections::HashSet<String>>>, String);

impl Claim {
    fn take(set: &'static std::sync::Mutex<Option<std::collections::HashSet<String>>>, id: &str) -> Option<Claim> {
        let fresh = set.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(id.to_string());
        fresh.then(|| Claim(set, id.to_string()))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(s) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            s.remove(&self.1);
        }
    }
}

pub async fn print(state: &Shared, id: &str, opts: PrintOptions) -> anyhow::Result<Job> {
    // Two taps (or the app and an automation) at once: the second waits for
    // nothing and is refused, rather than uploading and starting twice. The
    // claim lasts until the job is Uploading or Waiting, which is busy.
    let Some(_claim) = Claim::take(&STARTING, id) else { bail!("this print is already being sent") };
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.is_busy() {
        bail!("job is busy ({:?})", job.state);
    }
    if job.files_removed {
        bail!("{FILES_REMOVED}");
    }
    let slice = job.slice.clone().ok_or_else(|| anyhow!("slice the model first"))?;
    let printer = find_printer(state, Some(&slice.printer_id)).await?;
    if let Some(m) = &opts.ams_mapping {
        check_mapping(&slice, m)?;
    }
    let status = state.printers.status(&printer.id).await;
    if status.connected {
        if status.is_busy() {
            bail!("{} is busy ({})", printer.name, status.state.unwrap_or_default());
        }
        return send(state, id, job, printer, slice, status, opts).await;
    }

    // Offline: wait for the printer and send it then, unless waiting is off.
    let reason = status.last_error.map(|e| format!(" ({e})")).unwrap_or_default();
    let wait = state.printers.settings().wait_minutes;
    if wait == 0 {
        bail!("{} is not connected{reason}", printer.name);
    }
    // A plug that is off explains it: turn it on and wait for the printer.
    let mut powered = false;
    if crate::power::view(state, &printer).await.and_then(|v| v.on) == Some(false) {
        match crate::power::set(state, &printer, true).await {
            Ok(()) => powered = true,
            Err(e) => tracing::warn!("couldn't turn on {}'s plug: {e:#}", printer.name),
        }
    }
    remove_from_queue(state, id).await?;
    let job = update_job(state, id, |j| {
        j.state = JobState::Waiting;
        j.error = None;
        j.waiting_until = Some(now() + wait as i64 * 60);
        j.pending_print = Some(opts.clone());
        j.push_log(if powered {
            format!("Turned on {}'s plug. It will be sent when the printer has started (waiting up to {}).", printer.name, fmt_duration(wait as u64 * 60))
        } else {
            format!(
                "{} is offline{reason}. It will be sent when the printer is back (waiting up to {}).",
                printer.name,
                fmt_duration(wait as u64 * 60)
            )
        });
    })
    .await?;
    tokio::spawn(wait_and_send(state.clone(), id.to_string()));
    Ok(job)
}

// ------------------------------------------------------------------ follow

/// The printer's code for a print stopped by a person.
const CANCELLED_CODE: i64 = 0x0300_400C;

/// What a printer's report means for a started print.
#[derive(Debug, PartialEq)]
pub enum PrintUpdate {
    Running(i64),
    Ended(PrintOutcome, Option<i64>),
    Nothing,
}

/// Read a printer's status for the print with `task`.
pub fn print_update(status: &crate::printer::status::StatusView, task: &str) -> PrintUpdate {
    if !status.connected {
        return PrintUpdate::Nothing;
    }
    let reported = status.task_name.as_deref().map(|r| r.trim_end_matches(".3mf").trim_end_matches(".gcode"));
    let ours = reported == Some(task);
    let state = status.state.as_deref().unwrap_or("");
    match (ours, state) {
        (true, "FINISH") => PrintUpdate::Ended(PrintOutcome::Finished, None),
        (true, "FAILED") => match status.print_error {
            Some(CANCELLED_CODE) => PrintUpdate::Ended(PrintOutcome::Cancelled, None),
            code => PrintUpdate::Ended(PrintOutcome::Failed, code.filter(|c| *c != 0)),
        },
        (true, _) if status.is_busy() => PrintUpdate::Running(status.percent.unwrap_or(0)),
        // Something else is printing now: ours is over, result unknown.
        (false, _) if status.is_busy() => PrintUpdate::Ended(PrintOutcome::Unknown, None),
        _ => PrintUpdate::Nothing,
    }
}

/// Follow started prints until the printer says how they ended. Runs for
/// the life of the server; results are stored, live progress is not.
pub async fn watch_prints(state: Shared) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        crate::prints::notice_external(&state).await;
        let started: Vec<Job> = state
            .store
            .read(|d| d.jobs.iter().filter(|j| j.state == JobState::Started && j.print.is_some()).cloned().collect())
            .await;
        for job in started {
            let print = job.print.clone().unwrap();
            let status = state.printers.status(&print.printer_id).await;
            // A print the printer doesn't know about after a few minutes (it
            // never started, or the printer forgot it): result unknown.
            if status.connected
                && !status.is_busy()
                && !print.is_task(status.task_name.as_deref())
                && now() - print.started_at > 300
            {
                end_print(&state, &job.id, PrintOutcome::Unknown, None, &status).await;
                continue;
            }
            match print_update(&status, print.task_name()) {
                PrintUpdate::Running(p) => {
                    // Layer 2 started: the first layer is there to look at.
                    if !print.first_layer_done
                        && status.layer.unwrap_or(0) >= 2
                        && let Ok(j) = update_job(&state, &job.id, |j| {
                            if let Some(pr) = j.print.as_mut() {
                                pr.first_layer_done = true;
                            }
                        })
                        .await
                    {
                        crate::events::first_layer(&state, &j).await;
                    }
                    // Keep the last progress for partial filament use.
                    if print.progress != Some(p) && (p % 5 == 0 || print.progress.is_none()) {
                        let _ = update_job(&state, &job.id, |j| {
                            if let Some(pr) = j.print.as_mut() {
                                pr.progress = Some(p);
                                pr.layer = status.layer;
                                pr.total_layers = status.total_layers;
                            }
                        })
                        .await;
                    }
                }
                PrintUpdate::Ended(outcome, code) => end_print(&state, &job.id, outcome, code, &status).await,
                PrintUpdate::Nothing => {}
            }
        }
    }
}

async fn end_print(state: &Shared, id: &str, outcome: PrintOutcome, code: Option<i64>, status: &crate::printer::status::StatusView) {
    let printer_name = match get_job(state, id).await.and_then(|j| j.print).map(|p| p.printer_id) {
        Some(pid) => find_printer(state, Some(&pid)).await.map(|p| p.name).unwrap_or_else(|_| "the printer".into()),
        None => return,
    };
    let Ok(job) = update_job(state, id, |j| {
        let Some(pr) = j.print.as_mut() else { return };
        let ended = now();
        pr.outcome = Some(outcome);
        pr.ended_at = Some(ended);
        pr.error_code = code;
        if outcome == PrintOutcome::Finished {
            pr.progress = Some(100);
        } else if let Some(p) = status.percent.filter(|p| *p > 0) {
            pr.progress = Some(p);
        }
        // Only when the printer is still on this print: for an unknown end
        // it reports another print's layers.
        if outcome != PrintOutcome::Unknown {
            pr.layer = status.layer.or(pr.layer);
            pr.total_layers = status.total_layers.or(pr.total_layers);
        }
        let took = fmt_duration((ended - pr.started_at).max(0) as u64);
        match outcome {
            PrintOutcome::Finished => {
                j.state = JobState::Finished;
                j.push_log(format!("Printed on {printer_name} in {took}"));
            }
            PrintOutcome::Unknown => {
                j.state = JobState::Finished;
                j.push_log(format!("{printer_name} moved on to another print; this one's result isn't known"));
            }
            PrintOutcome::Cancelled => j.fail(format!("The print was stopped on {printer_name} after {took}")),
            PrintOutcome::Failed => j.fail(format!(
                "The print failed on {printer_name} after {took}{}",
                code.map(|c| format!(" (error {c:08X}) — check the printer screen")).unwrap_or_default()
            )),
        }
    })
    .await
    else {
        return;
    };
    crate::events::print_ended(state, &job).await;
}

/// Stop waiting for an offline printer; the job goes back to "sliced".
pub async fn cancel_wait(state: &Shared, id: &str) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state != JobState::Waiting {
        bail!("this print isn't waiting for the printer");
    }
    update_job(state, id, |j| {
        j.state = JobState::Sliced;
        j.waiting_until = None;
        j.pending_print = None;
        j.push_log("Stopped waiting for the printer");
    })
    .await
}

/// Watch an offline printer for a waiting job: send as soon as it is back,
/// give up at the deadline. Reconnect attempts are made every 15 s meanwhile.
///
/// Boxed with a named type because print() spawns it and it calls print().
fn wait_and_send(state: Shared, id: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(wait_and_send_inner(state, id))
}

async fn wait_and_send_inner(state: Shared, id: String) {
    const POLL: std::time::Duration = std::time::Duration::from_secs(15);
    // Already watched (waiting was stopped and started again while the
    // earlier loop slept): that loop reads the job afresh and carries on.
    let Some(_claim) = Claim::take(&WAITING, &id) else { return };
    loop {
        let Some(job) = get_job(&state, &id).await else { return };
        if job.state != JobState::Waiting {
            return; // cancelled, or deleted
        }
        let printer = match job.slice.as_ref().map(|s| s.printer_id.clone()) {
            Some(pid) => match find_printer(&state, Some(&pid)).await {
                Ok(p) => p,
                Err(e) => return fail(&state, &id, &e).await,
            },
            None => return fail(&state, &id, &anyhow!("the sliced file is gone")).await,
        };
        if state.printers.status(&printer.id).await.connected {
            // The printer sends its full state (trays included) right after
            // connecting; wait for that report before deciding anything.
            // A printer that just booted can miss the first request, so ask
            // again; without a fresh report, keep waiting rather than guess.
            let since = now();
            let mut fresh = false;
            for i in 0..10 {
                if state.printers.status(&printer.id).await.last_update.is_some_and(|t| t >= since) {
                    fresh = true;
                    break;
                }
                if i % 3 == 2 {
                    let _ = state.printers.request_full_status(&printer.id).await;
                }
                state.printers.changed(&printer.id, std::time::Duration::from_secs(1)).await;
            }
            let status = state.printers.status(&printer.id).await;
            if status.connected && fresh {
                if status.is_busy() {
                    let err = anyhow!(
                        "{} is back but busy ({}). Add it to the queue, or print it again when the printer is free.",
                        printer.name,
                        status.state.unwrap_or_default()
                    );
                    return fail(&state, &id, &err).await;
                }
                let opts = job.pending_print.clone().unwrap_or_default();
                let _ = update_job(&state, &id, |j| {
                    j.state = JobState::Sliced;
                    j.waiting_until = None;
                    j.pending_print = None;
                    j.push_log(format!("{} is back", printer.name));
                })
                .await;
                if let Err(e) = print(&state, &id, opts).await {
                    fail(&state, &id, &e).await;
                }
                // Sent, or failed: the next round ends the loop. Waiting again
                // (it dropped off right away): this loop keeps watching, as a
                // new one would be refused while this one holds the claim.
                continue;
            }
        }
        if job.waiting_until.is_some_and(|until| now() >= until) {
            let err = anyhow!("{} didn't come back online in time. Check that it's on and connected, then print again.", printer.name);
            return fail(&state, &id, &err).await;
        }
        state.printers.wake(&printer.id).await;
        state.printers.changed(&printer.id, POLL).await;
    }
}

/// Upload and start, with retries for the hiccups of a printer on Wi-Fi.
async fn send(
    state: &Shared,
    id: &str,
    job: Job,
    printer: Printer,
    slice: SliceOutcome,
    status: crate::printer::status::StatusView,
    opts: PrintOptions,
) -> anyhow::Result<Job> {
    let ams = match opts.ams_mapping.clone() {
        Some(m) => m,
        None => {
            // Nobody chose trays (an automatic print, or one that waited for
            // the printer): only go ahead when every filament has a match.
            let s = mapping::suggest_with(&slice.filaments, slice.filament_slots, &status, slice.preferred_trays.as_deref().unwrap_or(&[]));
            if !s.complete {
                bail!("couldn't match the filaments to the loaded trays: {} Open the print to choose trays.", s.warnings.join(" "));
            }
            s.ams_mapping
        }
    };
    if let Some(why) = status.drying_refusal(&ams) {
        bail!("{why}");
    }
    let use_ams = ams.iter().any(|&t| (0..crate::printer::status::EXTERNAL_TRAY).contains(&t));
    let local = slice.output_path.clone();
    let local = if local.as_os_str().is_empty() {
        job_dir(state, id).join(job.output_file.as_deref().ok_or_else(|| anyhow!("sliced file is missing"))?)
    } else {
        local
    };
    // The name tells slices apart: the printer only shows name and size,
    // and "already there" must mean this very slice (starting it again skips
    // the upload; a new slice of the same plate, even of equal size, doesn't).
    let modified = tokio::fs::metadata(&local).await.ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    let tag = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(format!("{id}|{modified}").as_bytes()))[..6].to_string()
    };
    // Added after shortening, so a long title can't cut it off.
    let named = ftp::safe_remote_name(&match &job.plate_label {
        Some(l) => format!("{} {l}", job.model_title),
        None => job.model_title.clone(),
    });
    let remote = format!("{}_{tag}.3mf", named.trim_end_matches(".3mf"));

    remove_from_queue(state, id).await?;
    let job = update_job(state, id, |j| {
        j.state = JobState::Uploading;
        j.error = None;
        j.push_log(format!("Uploading to {} as {remote}", printer.name));
    })
    .await?;

    let st = state.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        let result: anyhow::Result<()> = async {
            let settings = st.printers.settings();
            let target = ftp::FtpTarget { host: &printer.host, access_code: &printer.access_code };
            // Sending the same file again (a restart): if the printer still
            // has it, skip the upload.
            let local_size = tokio::fs::metadata(&local).await.map(|m| m.len()).ok();
            let on_printer = match ftp::remote_size(&target, &remote).await {
                Ok(size) => size.is_some() && size == local_size,
                Err(e) => {
                    tracing::debug!("checking for {remote} on {}: {e:#}", printer.name);
                    false
                }
            };
            let mut attempt = 1;
            let report = |sent: u64, total: u64| {
                let percent = (sent * 100).checked_div(total).unwrap_or(0).min(100) as u8;
                let message = format!("Uploading to {} ({:.0} of {:.0} MB)", printer.name, sent as f64 / 1e6, total as f64 / 1e6);
                st.set_progress(&id, Some(crate::models::SliceProgress { percent, message }));
            };
            if on_printer {
                log(&st, &id, format!("{} already has this file, so it isn't uploaded again", printer.name)).await;
            }
            if !on_printer {
                loop {
                    match ftp::upload(&target, printer.ftp_data_mode, printer.is_a1_family(), &local, &remote, Some(&report)).await {
                        Ok(()) => break,
                        Err(e) if attempt < settings.upload_attempts && worth_retrying(&e) => {
                            let pause = settings.retry_pause(attempt);
                            attempt += 1;
                            log(
                                &st,
                                &id,
                                format!("Upload failed ({e:#}). Trying again in {} s, attempt {attempt} of {}", pause.as_secs(), settings.upload_attempts),
                            )
                            .await;
                            st.printers.wake(&printer.id).await;
                            tokio::time::sleep(pause).await;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            st.set_progress(&id, None);
            update_job(&st, &id, |j| {
                j.state = JobState::Starting;
                j.push_log(if on_printer { "Starting print" } else { "Upload done, starting print" });
            })
            .await?;
            // The file is on the printer. If the status connection dropped
            // meanwhile, wait for it rather than failing: the command was
            // never sent, so sending it later can't start the print twice.
            let mut attempt = 1;
            loop {
                match st.printers.start_print(&printer, &remote, &slice.gcode_entry, &ams, &opts).await {
                    Ok(()) => break,
                    Err(e) if attempt < settings.upload_attempts && e.to_string() == crate::printer::NOT_CONNECTED => {
                        let pause = settings.retry_pause(attempt);
                        attempt += 1;
                        log(&st, &id, format!("{} isn't connected. Trying again in {} s, attempt {attempt} of {}", printer.name, pause.as_secs(), settings.upload_attempts)).await;
                        st.printers.wake(&printer.id).await;
                        st.printers.changed(&printer.id, pause).await;
                    }
                    Err(e) => return Err(e),
                }
            }
            let energy_start = crate::power::energy_kwh(&st, &printer).await;
            update_job(&st, &id, |j| {
                j.state = JobState::Started;
                j.print = Some(PrintStart {
                    printer_id: printer.id.clone(),
                    remote_file: remote.clone(),
                    ams_mapping: ams.clone(),
                    use_ams,
                    started_at: now(),
                    seen_running: true,
                    outcome: None,
                    ended_at: None,
                    error_code: None,
                    progress: None,
                    first_layer_done: false,
                    energy_start,
                    energy_kwh: None,
                    layer: None,
                    total_layers: None,
                    waste: None,
                });
                j.push_log(format!("Print started on {}", printer.name));
            })
            .await?;
            if let Some(job) = get_job(&st, &id).await {
                crate::events::print_started(&st, &job).await;
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            fail(&st, &id, &e).await;
        }
    });
    Ok(job)
}

/// Errors that a second try can fix (the printer was briefly unreachable),
/// as opposed to a wrong access code or a refusal (e.g. a full SD card).
fn worth_retrying(e: &anyhow::Error) -> bool {
    let m = format!("{e:#}").to_lowercase();
    if m.contains("could not reach") || m.contains("timed out") || m.contains("closed") || m.contains("interrupted") || m.contains("handshake") {
        return true;
    }
    !(m.contains("access code") || m.contains("refused") || m.contains("did not confirm") || m.contains("opening "))
}

fn check_mapping(slice: &SliceOutcome, m: &[i32]) -> anyhow::Result<()> {
    if (m.len() as u32) < slice.filament_slots {
        bail!("filament mapping needs {} entries", slice.filament_slots);
    }
    for f in &slice.filaments {
        if m.get(f.id as usize - 1).copied().unwrap_or(-1) < 0 {
            bail!("filament {} ({}) has no tray assigned", f.id, f.kind);
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ queue

/// Remove a job's queue entries; a queued job goes back to "sliced".
async fn remove_from_queue(state: &Shared, job_id: &str) -> anyhow::Result<()> {
    state
        .store
        .update(|d| {
            d.queue.retain(|q| q.job_id != job_id);
            if let Some(j) = d.jobs.iter_mut().find(|j| j.id == job_id && j.state == JobState::Queued) {
                j.state = JobState::Sliced;
            }
        })
        .await
}

/// Put a sliced job in its printer's queue.
pub async fn enqueue(state: &Shared, id: &str, opts: PrintOptions) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    let slice = job.slice.clone().ok_or_else(|| anyhow!("slice the model first"))?;
    if let Some(m) = &opts.ams_mapping {
        check_mapping(&slice, m)?;
    }
    let printer = find_printer(state, Some(&slice.printer_id)).await?;
    let item = QueueItem {
        id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
        job_id: id.to_string(),
        printer_id: printer.id.clone(),
        options: opts,
        added_at: now(),
    };
    let position = state
        .store
        .update(|d| {
            d.queue.retain(|q| q.job_id != item.job_id);
            d.queue.push(item.clone());
            let group = queue_group(&d.printers, &item.printer_id);
            d.queue.iter().filter(|q| queue_group(&d.printers, &q.printer_id) == group).count()
        })
        .await?;
    update_job(state, id, |j| {
        j.state = JobState::Queued;
        j.error = None;
        j.push_log(format!("Queued on {} (position {position})", printer.name));
    })
    .await
}

pub async fn dequeue(state: &Shared, item_id: &str) -> anyhow::Result<()> {
    let job_id = state
        .store
        .read(|d| d.queue.iter().find(|q| q.id == item_id).map(|q| q.job_id.clone()))
        .await
        .ok_or_else(|| anyhow!("not in the queue"))?;
    remove_from_queue(state, &job_id).await?;
    log(state, &job_id, "Removed from the queue").await;
    Ok(())
}

/// Move an entry up (-1) or down (+1) within its printer's queue.
/// The queue a printer's prints share: its printer profile (model and
/// nozzle), since a print sliced for one fits every printer with it.
pub fn queue_group(printers: &[Printer], printer_id: &str) -> String {
    printers.iter().find(|p| p.id == printer_id).map(|p| p.machine_profile.clone()).unwrap_or_else(|| printer_id.to_string())
}

pub async fn move_in_queue(state: &Shared, item_id: &str, delta: i32) -> anyhow::Result<()> {
    state
        .store
        .update(|d| {
            let Some(pos) = d.queue.iter().position(|q| q.id == item_id) else { bail!("not in the queue") };
            let group = queue_group(&d.printers, &d.queue[pos].printer_id);
            let same: Vec<usize> = d.queue.iter().enumerate().filter(|(_, q)| queue_group(&d.printers, &q.printer_id) == group).map(|(i, _)| i).collect();
            let idx = same.iter().position(|&i| i == pos).unwrap() as i32;
            let target = idx + delta;
            if target >= 0 && (target as usize) < same.len() {
                d.queue.swap(pos, same[target as usize]);
            }
            Ok(())
        })
        .await?
}

/// Start the first queued print of a printer's type on it. Call this once
/// the plate is clear.
pub async fn start_next(state: &Shared, printer_id: &str) -> anyhow::Result<Job> {
    let item = state
        .store
        .read(|d| {
            let group = queue_group(&d.printers, printer_id);
            d.queue.iter().find(|q| queue_group(&d.printers, &q.printer_id) == group).map(|q| q.id.clone())
        })
        .await
        .ok_or_else(|| anyhow!("nothing is queued for this type of printer"))?;
    start_queued(state, &item, printer_id).await
}

/// Start a queued print on a printer of the type it was sliced for (the
/// one it was queued on, or any other that is free). Moving it there keeps
/// the slice; trays are matched again on the new printer.
pub async fn start_queued(state: &Shared, item_id: &str, printer_id: &str) -> anyhow::Result<Job> {
    let (item, printers) = state.store.read(|d| (d.queue.iter().find(|q| q.id == item_id).cloned(), d.printers.clone())).await;
    let mut item = item.ok_or_else(|| anyhow!("not in the queue"))?;
    let target = printers.iter().find(|p| p.id == printer_id).cloned().ok_or_else(|| anyhow!("printer not found"))?;
    let job = get_job(state, &item.job_id).await.ok_or_else(|| anyhow!("queued job is gone"))?;
    let slice = job.slice.clone().ok_or_else(|| anyhow!("“{}” isn't sliced", job.model_title))?;
    if item.printer_id != target.id {
        let wanted = queue_group(&printers, &item.printer_id);
        if wanted != target.machine_profile {
            bail!("“{}” is sliced for {wanted}, not {}", job.model_title, target.machine_profile);
        }
        // The G-code is for one build plate; the other printer must have it.
        if let Some(bed) = &slice.bed_type
            && !target.plates.is_empty()
            && !target.plates.iter().any(|p| p.eq_ignore_ascii_case(bed))
        {
            bail!("“{}” is sliced for the {bed}, which {} doesn't have. Slice it again for {}.", job.model_title, target.name, target.name);
        }
        let (from, to) = (item.printer_id.clone(), target.id.clone());
        // Trays chosen on the other printer mean nothing here.
        item.options.ams_mapping = None;
        let opts = item.options.clone();
        state
            .store
            .update(|d| {
                if let Some(q) = d.queue.iter_mut().find(|q| q.id == item_id) {
                    q.printer_id = to.clone();
                    q.options = opts;
                }
            })
            .await?;
        let from_name = printers.iter().find(|p| p.id == from).map(|p| p.name.clone()).unwrap_or_default();
        update_job(state, &job.id, |j| {
            if let Some(s) = j.slice.as_mut() {
                s.printer_id = to.clone();
            }
            j.push_log(format!("Moved from {from_name} to {}", target.name));
        })
        .await?;
    }
    // Without a mapping chosen by a person, only start when the loaded
    // filament matches what the job needs.
    if item.options.ams_mapping.is_none() {
        let status = state.printers.status(&target.id).await;
        let s = mapping::suggest_with(&slice.filaments, slice.filament_slots, &status, slice.preferred_trays.as_deref().unwrap_or(&[]));
        if !s.complete {
            bail!("“{}” needs filament that isn't loaded on {}: {} Open it to choose trays.", job.model_title, target.name, s.warnings.join(" "));
        }
    }
    // print() refuses (and leaves the queue alone) when the printer is busy
    // or offline; on success it removes the entry.
    print(state, &item.job_id, item.options.clone()).await
}

/// A queued print, sliced again for a printer of another type (with the
/// same plate, objects and settings), then queued there. Trays are left to
/// match on that printer; process profile and plate are its own unless the
/// slice's build plate is one it has.
pub async fn reslice_queued(state: &Shared, item_id: &str, printer_id: &str) -> anyhow::Result<Job> {
    let (item, printers) = state.store.read(|d| (d.queue.iter().find(|q| q.id == item_id).cloned(), d.printers.clone())).await;
    let item = item.ok_or_else(|| anyhow!("not in the queue"))?;
    let target = printers.iter().find(|p| p.id == printer_id).cloned().ok_or_else(|| anyhow!("printer not found"))?;
    let job = get_job(state, &item.job_id).await.ok_or_else(|| anyhow!("queued job is gone"))?;
    let sliced = job.slice.clone().ok_or_else(|| anyhow!("“{}” isn't sliced", job.model_title))?;
    let group = queue_group(&printers, &item.printer_id);
    if group == target.machine_profile {
        bail!("“{}” is already sliced for {group}; start it on {} instead", job.model_title, target.name);
    }
    if sliced.settings == "presliced" || job.inspection.as_ref().is_some_and(|i| i.kind == "sliced") {
        bail!("“{}” came sliced for {group} and can't be sliced for another printer", job.model_title);
    }
    let asked = sliced.options.clone().unwrap_or_else(|| SliceOptions {
        plate: Some(sliced.plate),
        settings: Some(sliced.settings.clone()),
        transform: sliced.transform.clone().unwrap_or_default(),
        bed_type: sliced.bed_type.clone(),
        tweaks: sliced.tweaks.clone(),
        ..Default::default()
    });
    let opts = SliceOptions {
        printer_id: Some(target.id.clone()),
        // A project's own settings are for its printer; "auto" uses them
        // only when the project was made for this one.
        settings: asked.settings.clone().filter(|s| s == "printer"),
        // Profiles and trays belong to the other printer.
        process_profile: None,
        filament_profile: None,
        tray_id: None,
        tray_ids: None,
        bed_type: asked.bed_type.clone().filter(|b| target.plates.is_empty() || target.plates.iter().any(|p| p.eq_ignore_ascii_case(b))),
        extras: asked.extras.iter().map(|e| ExtraChoice { tray_id: None, ..e.clone() }).collect(),
        ..asked
    };
    let from = printers.iter().find(|p| p.id == item.printer_id).map(|p| p.name.clone()).unwrap_or_default();
    update_job(state, &job.id, |j| {
        j.auto_queue = true;
        j.push_log(format!("Slicing again for {} ({}), was queued for {from}", target.name, target.machine_profile));
    })
    .await?;
    // slice() takes it out of this queue; once sliced it joins the new one.
    match slice(state, &job.id, opts).await {
        Ok(j) => Ok(j),
        Err(e) => {
            let _ = update_job(state, &job.id, |j| j.auto_queue = false).await;
            Err(e)
        }
    }
}

/// Slice several plates of a project and queue them in plate order.
/// The job itself becomes the first plate; the others are copies.
pub async fn slice_all(state: &Shared, id: &str, opts: SliceOptions, plates: Option<Vec<u32>>) -> anyhow::Result<Vec<Job>> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    let insp = job.inspection.clone().ok_or_else(|| anyhow!("file is not downloaded yet"))?;
    let input = job.input_file.clone().ok_or_else(|| anyhow!("file is missing"))?;
    let wanted: Vec<u32> = match plates {
        Some(p) if !p.is_empty() => insp.plates.iter().map(|x| x.index).filter(|i| p.contains(i)).collect(),
        _ => insp.plates.iter().map(|x| x.index).collect(),
    };
    if wanted.is_empty() {
        bail!("no plates to slice");
    }
    if wanted.len() > 1 && !opts.extras.is_empty() {
        bail!("Added models go on one plate. Queue one plate at a time, or remove them first.");
    }
    // Make sure a printer is known before creating copies.
    let printer = find_printer(state, opts.printer_id.as_deref()).await?;
    let label = |p: u32| {
        let name = insp.plates.iter().find(|x| x.index == p).and_then(|x| x.name.clone());
        match name {
            Some(n) => format!("plate {p}: {n}"),
            None => format!("plate {p}"),
        }
    };

    let mut ids = Vec::new();
    for (i, plate) in wanted.iter().enumerate() {
        let jid = if i == 0 {
            update_job(state, id, |j| {
                j.plate_label = (wanted.len() > 1).then(|| label(*plate));
                j.auto_queue = true;
                j.auto = false;
            })
            .await?;
            id.to_string()
        } else {
            let mut copy = new_job(job.source, job.model_url.clone(), job.model_id.clone(), job.model_title.clone(), job.model_cover.clone(), job.file.clone());
            copy.copied_from = Some(job.id.clone());
            copy.input_file = Some(input.clone());
            copy.inspection = Some(insp.clone());
            copy.state = JobState::Ready;
            copy.plate_label = Some(label(*plate));
            copy.auto_queue = true;
            copy.favorite = false;
            let to = job_dir(state, &copy.id);
            tokio::fs::create_dir_all(&to).await?;
            tokio::fs::copy(job_dir(state, id).join(&input), to.join(&input)).await.context("copying the model")?;
            copy.push_log(format!("Part of “Queue all plates” for {}", job.model_title));
            insert_job(state, &copy).await?;
            copy.id
        };
        ids.push((jid, *plate));
    }
    let mut out = Vec::new();
    for (jid, plate) in ids {
        let o = SliceOptions { plate: Some(plate), printer_id: Some(printer.id.clone()), ..opts.clone() };
        out.push(slice(state, &jid, o).await?);
    }
    Ok(out)
}

/// Jobs made from the same downloaded or uploaded file.
fn same_file(a: &Job, b: &Job) -> bool {
    a.source == b.source && a.model_url == b.model_url && a.file.id == b.file.id && a.file.name == b.file.name
}

/// The plate a job prints: the sliced one, or the one it is being sliced for.
fn job_plate(j: &Job) -> Option<u32> {
    // With added models the sliced file has one plate; the options say which it was.
    j.slice
        .as_ref()
        .map(|s| s.options.as_ref().filter(|o| !o.extras.is_empty()).and_then(|o| o.plate).unwrap_or(s.plate))
        .or_else(|| j.auto_slice.as_ref().and_then(|o| o.plate))
}

/// For a job that went to the printer: the first plate of its file that no
/// job has printed, queued or is getting ready to queue.
pub async fn next_plate(state: &Shared, job: &Job) -> Option<PlateInfo> {
    let insp = job.inspection.as_ref()?;
    let sent = job.print.is_some() || matches!(job.state, JobState::Waiting | JobState::Uploading | JobState::Starting | JobState::Started | JobState::Finished);
    if insp.plates.len() < 2 || !sent || job.files_removed {
        return None;
    }
    let taken: Vec<u32> = state
        .store
        .read(|d| {
            d.jobs
                .iter()
                .filter(|j| same_file(j, job))
                .filter(|j| {
                    j.print.is_some()
                        || (j.auto_queue && matches!(j.state, JobState::Ready | JobState::Slicing))
                        || matches!(j.state, JobState::Queued | JobState::Waiting | JobState::Uploading | JobState::Starting | JobState::Started | JobState::Finished)
                })
                .filter_map(job_plate)
                .collect()
        })
        .await;
    insp.plates.iter().find(|p| !taken.contains(&p.index)).cloned()
}

/// For the start page: with an empty queue, the latest print whose file
/// still has a plate nobody printed or queued.
pub async fn suggest_next_plate(state: &Shared) -> Option<(Job, PlateInfo)> {
    let (queue_empty, last) = state
        .store
        .read(|d| {
            let last = d.jobs.iter().filter(|j| j.print.is_some()).max_by_key(|j| j.print.as_ref().map(|p| p.started_at)).cloned();
            (d.queue.is_empty() && !d.jobs.iter().any(|j| j.auto_queue && j.state.is_busy()), last)
        })
        .await;
    let last = last.filter(|_| queue_empty)?;
    let plate = next_plate(state, &last).await?;
    Some((last, plate))
}

/// Slice the next unprinted plate of a printed job's file with the same
/// settings and queue it on the same printer.
pub async fn queue_next_plate(state: &Shared, id: &str) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    let sliced = job.slice.clone().ok_or_else(|| anyhow!("this job wasn't sliced"))?;
    let plate = next_plate(state, &job).await.ok_or_else(|| anyhow!("every plate of this file is printed or queued"))?;
    let input = job.input_file.clone().ok_or_else(|| anyhow!("file is missing"))?;
    let label = match &plate.name {
        Some(n) => format!("plate {}: {n}", plate.index),
        None => format!("plate {}", plate.index),
    };
    let opts = SliceOptions {
        printer_id: Some(sliced.printer_id.clone()),
        plate: Some(plate.index),
        settings: (sliced.settings != "presliced").then(|| sliced.settings.clone()),
        process_profile: None,
        filament_profile: None,
        transform: Default::default(),
        tray_id: None,
        tray_ids: sliced.preferred_trays.clone(),
        objects: None,
        bed_type: sliced.bed_type.clone(),
        tweaks: sliced.tweaks.clone(),
        extras: vec![],
    };
    let mut copy = new_job(job.source, job.model_url.clone(), job.model_id.clone(), job.model_title.clone(), job.model_cover.clone(), job.file.clone());
    copy.copied_from = Some(job.id.clone());
    copy.input_file = Some(input.clone());
    copy.inspection = job.inspection.clone();
    copy.state = JobState::Ready;
    copy.plate_label = Some(label.clone());
    copy.auto_queue = true;
    copy.auto_slice = Some(opts.clone());
    let to = job_dir(state, &copy.id);
    tokio::fs::create_dir_all(&to).await?;
    tokio::fs::copy(job_dir(state, id).join(&input), to.join(&input)).await.context("the original file is gone")?;
    // Name the printed plate too, so the two tell apart in the history.
    let this = match &job.plate_label {
        Some(l) => l.clone(),
        None => {
            let name = job.inspection.as_ref().and_then(|i| i.plates.iter().find(|p| p.index == sliced.plate)).and_then(|p| p.name.clone());
            let l = match name {
                Some(n) => format!("plate {}: {n}", sliced.plate),
                None => format!("plate {}", sliced.plate),
            };
            let _ = update_job(state, id, |j| j.plate_label = Some(l.clone())).await;
            l
        }
    };
    copy.push_log(format!("Next plate after {this}: {label}"));
    insert_job(state, &copy).await?;
    match slice(state, &copy.id, opts).await {
        Ok(j) => Ok(j),
        Err(e) => {
            fail(state, &copy.id, &e).await;
            Err(e)
        }
    }
}

// ------------------------------------------------------------------ favourites and reprint

pub async fn set_favorite(state: &Shared, id: &str, favorite: bool) -> anyhow::Result<Job> {
    update_job(state, id, |j| j.favorite = favorite).await
}

/// Copy a job's files into a new job so it can be printed again without
/// downloading or slicing. Goes straight to filament check when it was sliced.
pub async fn reprint(state: &Shared, id: &str) -> anyhow::Result<Job> {
    let old = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if old.state.is_busy() {
        bail!("job is busy ({:?})", old.state);
    }
    if old.source == Source::External {
        bail!("This print wasn't started from AirSlice, so AirSlice has no file to print again.");
    }
    let from = job_dir(state, &old.id);
    let gone = match &old.input_file {
        _ if old.files_removed => true,
        Some(f) => !tokio::fs::try_exists(from.join(f)).await.unwrap_or(false),
        None => false,
    };
    if gone {
        // Fetch it again from where it came from.
        if old.source == Source::Upload {
            bail!("The file of this job is gone. Upload it again to print it.");
        }
        let mut job = create(state, &old.model_url, Some(&old.file.id), None).await?;
        job = update_job(state, &job.id, |j| {
            j.copied_from = Some(old.id.clone());
            j.push_log("Downloading again: the earlier files are gone");
            if !old.extras.is_empty() {
                j.push_log("Models added to the plate before aren't downloaded again. Add them again if you want them.");
            }
        })
        .await?;
        if !old.files_removed {
            let _ = update_job(state, &old.id, |j| j.files_removed = true).await;
        }
        return Ok(job);
    }
    let input = old.input_file.clone().ok_or_else(|| anyhow!("this job has no file to print again"))?;
    let mut job = new_job(old.source, old.model_url.clone(), old.model_id.clone(), old.model_title.clone(), old.model_cover.clone(), old.file.clone());
    job.copied_from = Some(old.id.clone());
    let to = job_dir(state, &job.id);
    tokio::fs::create_dir_all(&to).await?;
    tokio::fs::copy(from.join(&input), to.join(&input)).await.context("the original file is gone")?;
    job.input_file = Some(input.clone());
    job.inspection = old.inspection.clone();
    job.state = JobState::Ready;
    // Added models come along, so slicing it again gives the same plate.
    if !old.extras.is_empty() {
        tokio::fs::create_dir_all(to.join("extras")).await?;
        for extra in &old.extras {
            if tokio::fs::copy(from.join("extras").join(&extra.input_file), to.join("extras").join(&extra.input_file)).await.is_ok() {
                job.extras.push(extra.clone());
            }
        }
    }
    if let (Some(mut slice), Some(out)) = (old.slice.clone(), old.output_file.clone()) {
        let copied = if out == input {
            true
        } else {
            tokio::fs::copy(from.join(&out), to.join(&out)).await.is_ok()
        };
        if copied {
            slice.output_path = to.join(&out);
            job.slice = Some(slice);
            job.output_file = Some(out);
            job.state = JobState::Sliced;
        }
    }
    job.push_log(format!(
        "Copied from an earlier job{}",
        if job.state == JobState::Sliced { " — already sliced" } else { "" }
    ));
    insert_job(state, &job).await?;
    Ok(job)
}

/// Go back to plate and settings choice (re-slice with other options).
pub async fn reslice(state: &Shared, id: &str) -> anyhow::Result<Job> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("{}", busy_text(job.state));
    }
    if job.inspection.is_none() {
        bail!("this job has no file yet");
    }
    remove_from_queue(state, id).await?;
    update_job(state, id, |j| {
        j.state = JobState::Ready;
        j.slice = None;
        j.error = None;
        j.push_log("Back to plate and settings");
    })
    .await
}

/// Delete a job and its files.
pub async fn delete(state: &Shared, id: &str) -> anyhow::Result<()> {
    let job = get_job(state, id).await.ok_or_else(|| anyhow!("job not found"))?;
    if job.state.in_use() {
        bail!("the job is still running");
    }
    state
        .store
        .update(|d| {
            d.jobs.retain(|j| j.id != id);
            d.queue.retain(|q| q.job_id != id);
        })
        .await?;
    let _ = tokio::fs::remove_dir_all(job_dir(state, id)).await;
    let _ = tokio::fs::remove_file(state.cfg.data_dir.join("photos").join(format!("{id}.jpg"))).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FtpDataMode;

    #[test]
    fn trims_the_picture_cache_oldest_first() {
        let dir = std::env::temp_dir().join(format!("as-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now();
        let day = std::time::Duration::from_secs(86_400);
        for (name, age_days) in [("new", 0), ("week", 7), ("month", 20), ("ancient", 40)] {
            let p = dir.join(name);
            std::fs::write(&p, vec![0u8; 1000]).unwrap();
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(now - day * age_days).unwrap();
        }
        // 2500 bytes allowed: "ancient" is too old, "month" doesn't fit.
        assert_eq!(trim_cache(&dir, 2500, day * 30), (2, 2000));
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["new", "week"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extras_share_slots_where_trays_match() {
        // The base uses trays 2 and 5. Tray 5 shares its slot, 7 used twice
        // gets one new slot, no tray takes the first.
        let (slots, new) = extra_slots(&[2, 5], 2, &[Some(5), Some(7), Some(7), None, Some(-1), Some(9)]);
        assert_eq!(slots, [2, 3, 3, 1, 1, 4]);
        assert_eq!(new, [7, 9]);
        // No trays chosen for the base: any tray is a new slot.
        let (slots, new) = extra_slots(&[], 4, &[Some(1)]);
        assert_eq!(slots, [5]);
        assert_eq!(new, [1]);
    }

    #[tokio::test]
    async fn recover_flags_jobs_whose_files_are_gone() {
        let dir = std::env::temp_dir().join(format!("as-recover-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let cfg = crate::config::Config { data_dir: dir.clone(), ..crate::config::Config::from_env().unwrap() };
        let store = crate::store::Store::open(&dir.join("state.json")).await.unwrap();
        let state: Shared = std::sync::Arc::new(crate::state::AppState::new(cfg, store, crate::printer::PrinterManager::new(), reqwest::Client::new()));
        let file = ModelFile {
            id: "f".into(),
            name: "egg.3mf".into(),
            kind: "3mf".into(),
            size_bytes: None,
            thumbnail: None,
            printer: None,
            compatible: vec![],
            plates: None,
            need_ams: None,
            filaments: vec![],
            download_type: None,
            folder: None,
            prediction_secs: None,
            weight_g: None,
        };
        let mut kept = new_job(Source::Makerworld, "u".into(), "m".into(), "kept".into(), None, file.clone());
        let mut lost = new_job(Source::Makerworld, "u".into(), "m".into(), "lost".into(), None, file);
        for j in [&mut kept, &mut lost] {
            j.state = JobState::Finished;
            j.input_file = Some("input.3mf".into());
        }
        tokio::fs::create_dir_all(job_dir(&state, &kept.id)).await.unwrap();
        tokio::fs::write(job_dir(&state, &kept.id).join("input.3mf"), b"x").await.unwrap();
        insert_job(&state, &kept).await.unwrap();
        insert_job(&state, &lost).await.unwrap();
        recover(&state).await.unwrap();
        assert!(!get_job(&state, &kept.id).await.unwrap().files_removed);
        assert!(get_job(&state, &lost.id).await.unwrap().files_removed);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    fn printer(model: &str) -> Printer {
        Printer {
            id: "p".into(),
            name: "P".into(),
            host: "1.2.3.4".into(),
            serial: "S".into(),
            access_code: "x".into(),
            printer_model: model.into(),
            machine_profile: format!("{model} 0.4 nozzle"),
            process_profile: Some("proc".into()),
            filament_profile: None,
            ftp_data_mode: FtpDataMode::Auto,
            plates: vec![],
            default_plate: None,
            camera: String::new(),
            camera_entity: None,
            power_switch: None,
            power_sensor: None,
            energy_sensor: None,
            power_off_after_min: 0,
            enabled: true,
            created_at: 0,
        }
    }

    fn insp(kind: &str, model: Option<&str>) -> Inspection {
        Inspection {
            kind: kind.into(),
            plates: vec![PlateInfo { index: 1, name: None, has_thumbnail: false, object_count: 1, objects: vec![], prediction_secs: None, weight_g: None }],
            printer_model: model.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn plans() {
        let p = printer("Bambu Lab P1S");
        let o = SliceOptions::default();
        assert_eq!(plan(&insp("sliced", None), &p, &o), Plan::Presliced);
        assert_eq!(plan(&insp("project", Some("Bambu Lab P1S")), &p, &o), Plan::Project { copies: 1 });
        assert_eq!(
            plan(&insp("project", Some("Bambu Lab A1")), &p, &o),
            Plan::Printer { process: Some("proc".into()), filament: None, arrange: false, transform: MeshTransform::default() }
        );
        assert!(matches!(plan(&insp("mesh", None), &p, &o), Plan::Printer { arrange: true, .. }));
        let force = SliceOptions { settings: Some("project".into()), ..Default::default() };
        assert_eq!(plan(&insp("project", Some("Bambu Lab A1")), &p, &force), Plan::Project { copies: 1 });

        // Copies: pre-sliced files are sliced again, projects keep their plan.
        let three = SliceOptions { transform: MeshTransform { copies: Some(3), ..Default::default() }, ..Default::default() };
        assert_eq!(plan(&insp("sliced", None), &p, &three), Plan::Project { copies: 3 });
        assert_eq!(plan(&insp("project", Some("Bambu Lab P1S")), &p, &three), Plan::Project { copies: 3 });
        assert!(matches!(
            plan(&insp("project", Some("Bambu Lab A1")), &p, &three),
            Plan::Printer { arrange: false, transform: MeshTransform { copies: Some(3), .. }, .. }
        ));
    }

    #[test]
    fn retries_only_what_can_heal() {
        let e = |m: &str| anyhow!(m.to_string());
        assert!(worth_retrying(&e("could not reach 10.0.0.5:990: Connection refused (os error 111)")));
        assert!(worth_retrying(&e("TLS handshake with the printer timed out")));
        assert!(worth_retrying(&e("printer closed the FTP connection")));
        assert!(worth_retrying(&anyhow!("No route to host").context("upload interrupted")));
        assert!(!worth_retrying(&e("printer rejected the access code (530 Login incorrect.)")));
        assert!(!worth_retrying(&e("printer refused the upload: 552 Disk full")));
    }

    #[test]
    fn retry_pauses_double() {
        let s = crate::models::ConnectionSettings { retry_delay_secs: 10, ..Default::default() };
        let secs: Vec<u64> = (1..=4).map(|n| s.retry_pause(n).as_secs()).collect();
        assert_eq!(secs, vec![10, 20, 40, 80]);
        assert_eq!(crate::models::ConnectionSettings { retry_delay_secs: 600, ..Default::default() }.retry_pause(5).as_secs(), 600);
        assert!(crate::models::ConnectionSettings::default().validate().is_ok());
        assert!(crate::models::ConnectionSettings { upload_attempts: 0, ..Default::default() }.validate().is_err());
    }

    #[test]
    fn reads_print_endings() {
        use crate::printer::status::StatusView;
        let st = |state: &str, task: &str, err: Option<i64>| StatusView {
            connected: true,
            state: Some(state.into()),
            task_name: Some(task.into()),
            print_error: err,
            percent: Some(40),
            ..Default::default()
        };
        assert_eq!(print_update(&st("RUNNING", "rex", None), "rex"), PrintUpdate::Running(40));
        assert_eq!(print_update(&st("FINISH", "rex", None), "rex"), PrintUpdate::Ended(PrintOutcome::Finished, None));
        assert_eq!(print_update(&st("FAILED", "rex", Some(0x0300_400C)), "rex"), PrintUpdate::Ended(PrintOutcome::Cancelled, None));
        assert_eq!(print_update(&st("FAILED", "rex", Some(0x0700_8011)), "rex"), PrintUpdate::Ended(PrintOutcome::Failed, Some(0x0700_8011)));
        assert_eq!(print_update(&st("RUNNING", "other", None), "rex"), PrintUpdate::Ended(PrintOutcome::Unknown, None));
        assert_eq!(print_update(&st("FINISH", "other", None), "rex"), PrintUpdate::Nothing);
        assert_eq!(print_update(&StatusView::default(), "rex"), PrintUpdate::Nothing);
    }

    #[test]
    fn counts_fitted_copies() {
        // Plate 1 of the T-rex has 3 objects; 8 copies asked, 6 objects landed.
        assert_eq!(fitted_copies(6, 3), 2);
        assert_eq!(fitted_copies(24, 3), 8);
        assert_eq!(fitted_copies(5, 1), 5);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(30), "1 min");
        assert_eq!(fmt_duration(3 * 3600 + 25 * 60), "3 h 25 min");
    }
}
