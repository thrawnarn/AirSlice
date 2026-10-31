//! Things that happen to jobs, for notifications (and Spoolman).

use crate::{
    i18n::{Lang, clock, money, t},
    jobs::fmt_duration,
    models::{Cost, Job, JobState, PrintOutcome, now},
    notify::{self, Pictures},
    state::Shared,
};
use std::sync::OnceLock;

/// The owner's time zone: `TZ` (the Supervisor sets it for add-ons), else
/// Home Assistant's own setting, else UTC.
async fn time_zone(state: &Shared) -> jiff::tz::TimeZone {
    static NAME: OnceLock<String> = OnceLock::new();
    let name = match NAME.get() {
        Some(n) => n.clone(),
        None => {
            let mut n = std::env::var("TZ").unwrap_or_default();
            if n.is_empty() {
                let s = state.store.read(|d| d.notify.clone()).await;
                if let Ok((base, token)) = notify::endpoint(&s)
                    && let Ok(r) = state.http.get(format!("{base}/config")).bearer_auth(token).send().await
                    && let Ok(v) = r.json::<serde_json::Value>().await
                {
                    n = v.get("time_zone").and_then(|t| t.as_str()).unwrap_or_default().to_string();
                }
            }
            let _ = NAME.set(n.clone());
            n
        }
    };
    jiff::tz::TimeZone::get(&name).unwrap_or(jiff::tz::TimeZone::UTC)
}

/// A moment in the owner's time zone, and now, for `i18n::clock`.
async fn zoned(state: &Shared, unix: i64) -> (jiff::Zoned, jiff::Zoned) {
    let tz = time_zone(state).await;
    let at = jiff::Timestamp::from_second(unix).unwrap_or_else(|_| jiff::Timestamp::now());
    (at.to_zoned(tz.clone()), jiff::Timestamp::now().to_zoned(tz))
}

fn kwh_text(l: Lang, kwh: f64) -> String {
    let n = format!("{kwh:.2}");
    if l == Lang::Sv { n.replace('.', ",") } else { n }
}

fn cost_text(l: Lang, c: &Cost) -> String {
    money(l, c.total, &c.currency)
}

/// The plate render for a job, if its file is still there.
fn plate_picture(state: &Shared, job: &Job) -> Option<(String, std::path::PathBuf, u32)> {
    let plate = job.slice.as_ref()?.plate;
    let file = job.input_file.as_ref().filter(|_| !job.files_removed)?;
    let has = job.inspection.as_ref()?.plates.iter().any(|p| p.index == plate && p.has_thumbnail);
    has.then(|| (job.id.clone(), crate::jobs::job_dir(state, &job.id).join(file), plate))
}

/// Estimated cost from the slice and the trays it was sent with.
async fn estimate(state: &Shared, job: &Job) -> Option<Cost> {
    let (slice, print) = (job.slice.as_ref()?, job.print.as_ref()?);
    let use_g = crate::spoolman::use_per_tray(&slice.filaments, &print.ams_mapping, 1.0);
    let prices = crate::spoolman::prices_for_printer(state, &print.printer_id).await;
    let hours = slice.prediction_secs.map(|s| s as f64 / 3600.0);
    let costs = state.store.read(|d| d.cost.clone()).await;
    crate::spoolman::cost(&use_g, &prices, hours, &costs, false)
}

fn title(job: &Job) -> String {
    match &job.plate_label {
        Some(l) => format!("{} ({l})", job.model_title),
        None => job.model_title.clone(),
    }
}

async fn printer_name(state: &Shared, job: &Job) -> String {
    let id = job.print.as_ref().map(|p| p.printer_id.clone()).or_else(|| job.slice.as_ref().map(|s| s.printer_id.clone()));
    match id {
        Some(id) => state.store.read(|d| d.printers.iter().find(|p| p.id == id).map(|p| p.name.clone())).await.unwrap_or_default(),
        None => String::new(),
    }
}

/// Sliced and waiting for a person (automatic queue/print flows say so later).
pub async fn sliced(state: &Shared, job: &Job) {
    if job.auto || job.auto_queue {
        return;
    }
    let name = title(job);
    let time = job.slice.as_ref().and_then(|s| s.prediction_secs).map(fmt_duration);
    let weight = job.slice.as_ref().and_then(|s| s.weight_g).map(|g| format!("{g:.0} g"));
    let text: notify::Text = Box::new(move |l| {
        let body = match (&time, &weight) {
            (Some(tm), Some(w)) => t(l, "{0} — {1}, {2}", &[&name, tm, w]),
            _ => name.clone(),
        };
        (t(l, "Sliced, ready to print", &[]), body)
    });
    notify::send_with_pictures(state, |s| s.sliced, text, Pictures { plate: plate_picture(state, job), ..Default::default() });
}

async fn printer_of(state: &Shared, job: &Job) -> Option<crate::models::Printer> {
    let id = job.print.as_ref()?.printer_id.clone();
    state.store.read(|d| d.printers.iter().find(|p| p.id == id).cloned()).await
}

pub async fn print_started(state: &Shared, job: &Job) {
    let printer = printer_name(state, job).await;
    let name = title(job);
    let started = job.print.as_ref().map(|p| p.started_at).unwrap_or_else(now);
    let plan = match job.slice.as_ref().and_then(|s| s.prediction_secs) {
        Some(secs) => Some((fmt_duration(secs), zoned(state, started + secs as i64).await)),
        None => None,
    };
    let weight = job.slice.as_ref().and_then(|s| s.weight_g).map(|g| format!("{g:.0} g"));
    let cost = estimate(state, job).await;
    let text: notify::Text = Box::new(move |l| {
        let mut lines = vec![name.clone()];
        if let Some((d, (at, today))) = &plan {
            lines.push(t(l, "About {0}, ready around {1}", &[d, &clock(l, at, today)]));
        }
        match (&cost, &weight) {
            (Some(c), Some(w)) => lines.push(t(l, "Estimated cost {0}, {1}", &[&cost_text(l, c), w])),
            (Some(c), None) => lines.push(t(l, "Estimated cost {0}", &[&cost_text(l, c)])),
            (None, Some(w)) => lines.push(w.clone()),
            _ => {}
        }
        (t(l, "Printing on {0}", &[&printer]), lines.join("\n"))
    });
    let pics = Pictures { camera: printer_of(state, job).await, plate: plate_picture(state, job), both: true };
    notify::send_with_pictures(state, |s| s.printing, text, pics);
}

/// The first layer is down: a picture shows whether it sticks.
pub async fn first_layer(state: &Shared, job: &Job) {
    let printer = printer_name(state, job).await;
    let p = printer_of(state, job).await;
    let name = title(job);
    let mut elapsed = None;
    let mut ready = None;
    if let Some(pr) = &job.print {
        elapsed = Some(fmt_duration((now() - pr.started_at).max(0) as u64));
        if let Some(m) = state.printers.status(&pr.printer_id).await.remaining_minutes.filter(|m| *m > 0) {
            ready = Some(zoned(state, now() + m * 60).await);
        }
    }
    let text: notify::Text = Box::new(move |l| {
        let mut lines = vec![t(l, "{0} — check that it sticks", &[&name])];
        match (&elapsed, &ready) {
            (Some(e), Some((at, today))) => lines.push(t(l, "Printing for {0}, ready around {1}", &[e, &clock(l, at, today)])),
            (Some(e), None) => lines.push(t(l, "Printing for {0}", &[e])),
            _ => {}
        }
        (t(l, "First layer done on {0}", &[&printer]), lines.join("\n"))
    });
    let pics = Pictures { camera: p, plate: plate_picture(state, job), both: false };
    notify::send_with_pictures(state, |s| s.first_layer, text, pics);
}

pub async fn print_ended(state: &Shared, job: &Job) {
    account(state, job).await;
    if let Some(pr) = &job.print
        && let Some(printer) = printer_of(state, job).await
    {
        // What the plug's meter counted during the print.
        if let (Some(start), Some(end)) = (pr.energy_start, crate::power::energy_kwh(state, &printer).await)
            && end > start
        {
            let used = end - start;
            let _ = crate::jobs::update_job(state, &job.id, |j| {
                if let Some(p) = j.print.as_mut() {
                    p.energy_kwh = Some(used);
                }
                j.push_log(format!("Used {used:.2} kWh"));
            })
            .await;
        }
        if printer.power_switch.is_some() && printer.power_off_after_min > 0 {
            crate::power::arm(&printer.id);
        }
    }
    // A picture of the result, while it's still on the plate.
    if let Some(pid) = job.print.as_ref().map(|p| p.printer_id.clone())
        && let Some(printer) = state.store.read(|d| d.printers.iter().find(|p| p.id == pid).cloned()).await
        && crate::camera::save_photo(state, &printer, &job.id).await
    {
        let _ = crate::jobs::update_job(state, &job.id, |j| j.photo = true).await;
    }
    let printer = printer_name(state, job).await;
    let printer_of_job = printer_of(state, job).await;
    // Re-read: `account` just stored the cost.
    let job = &crate::jobs::get_job(state, &job.id).await.unwrap_or_else(|| job.clone());
    let mut took = None;
    if let Some(p) = &job.print
        && let Some(end) = p.ended_at
    {
        let d = fmt_duration((end - p.started_at).max(0) as u64);
        let planned = job.slice.as_ref().and_then(|s| s.prediction_secs).map(fmt_duration).filter(|pl| *pl != d);
        took = Some((d, planned, zoned(state, end).await));
    }
    let cost = job.cost.clone();
    let energy = job.print.as_ref().and_then(|p| p.energy_kwh);
    let outcome = job.print.as_ref().and_then(|p| p.outcome);
    let (name, error) = (title(job), job.error.clone());
    let head = match outcome {
        Some(PrintOutcome::Finished) => "Printed on {0}",
        Some(PrintOutcome::Cancelled) => "Print stopped on {0}",
        Some(PrintOutcome::Failed) => "Print failed on {0}",
        _ => return,
    };
    let who = printer.clone();
    let text: notify::Text = Box::new(move |l| {
        // A failed print says why (the printer's own words, in English).
        let mut lines = vec![if outcome == Some(PrintOutcome::Failed) { error.clone().unwrap_or_else(|| name.clone()) } else { name.clone() }];
        match &took {
            Some((d, Some(pl), (at, today))) => lines.push(t(l, "Took {0} (estimated {1}), done {2}", &[d, pl, &clock(l, at, today)])),
            Some((d, None, (at, today))) => lines.push(t(l, "Took {0}, done {1}", &[d, &clock(l, at, today)])),
            None => {}
        }
        match (&cost, energy) {
            (Some(c), Some(kwh)) => lines.push(t(l, "Cost {0}, {1} kWh", &[&cost_text(l, c), &kwh_text(l, kwh)])),
            (Some(c), None) => lines.push(t(l, "Cost {0}", &[&cost_text(l, c)])),
            (None, Some(kwh)) => lines.push(t(l, "Used {0} kWh", &[&kwh_text(l, kwh)])),
            _ => {}
        }
        (t(l, head, &[&who]), lines.join("\n"))
    });
    let pics = Pictures { camera: printer_of_job.clone(), plate: plate_picture(state, job), both: true };
    notify::send_with_pictures(state, |s| s.finished, text, pics);
    // The printer is free: prints queued for its type wait for a person to
    // clear the plate (any printer of the type can take them).
    let Some(pid) = job.print.as_ref().map(|p| p.printer_id.clone()) else { return };
    let (next, waiting, total) = state
        .store
        .read(|d| {
            let group = crate::jobs::queue_group(&d.printers, &pid);
            let jobs: Vec<&Job> = d
                .queue
                .iter()
                .filter(|q| crate::jobs::queue_group(&d.printers, &q.printer_id) == group)
                .filter_map(|q| d.jobs.iter().find(|j| j.id == q.job_id))
                .collect();
            let total: u64 = jobs.iter().filter_map(|j| j.slice.as_ref()?.prediction_secs).sum();
            (jobs.first().map(|j| (*j).clone()), jobs.len(), total)
        })
        .await;
    if let Some(next) = next {
        let pics = Pictures { camera: printer_of_job, plate: plate_picture(state, &next), both: true };
        let next_name = title(&next);
        let next_time = next.slice.as_ref().and_then(|s| s.prediction_secs).map(fmt_duration);
        let all = (waiting > 1 && total > 0).then(|| fmt_duration(total));
        let text: notify::Text = Box::new(move |l| {
            let head = if waiting == 1 {
                t(l, "{0} is free — 1 print waiting", &[&printer])
            } else {
                t(l, "{0} is free — {1} prints waiting", &[&printer, &waiting.to_string()])
            };
            let mut lines = vec![match &next_time {
                Some(tm) => t(l, "Next: {0}, {1}", &[&next_name, tm]),
                None => t(l, "Next: {0}", &[&next_name]),
            }];
            if let Some(a) = &all {
                lines.push(t(l, "{0} in the queue", &[a]));
            }
            lines.push(t(l, "Clear the plate, then start it.", &[]));
            (head, lines.join("\n"))
        });
        notify::send_with_pictures(state, |s| s.queue, text, pics);
    }
}

/// A job stopped with an error (slicing, upload, waiting for the printer…).
pub async fn problem(state: &Shared, job: &Job) {
    if job.state != JobState::Failed {
        return;
    }
    // A picture of the printer helps when it went wrong there.
    let on_printer = job.print.is_some();
    let pics = Pictures { camera: if on_printer { printer_of(state, job).await } else { None }, plate: plate_picture(state, job), both: false };
    let (name, error) = (title(job), job.error.clone());
    let text: notify::Text = Box::new(move |l| {
        (t(l, "Problem with {0}", &[&name]), error.clone().unwrap_or_else(|| t(l, "Something went wrong", &[])))
    });
    notify::send_with_pictures(state, |s| s.problems, text, pics);
}

/// What the print cost and used: stored on the job, and booked in Spoolman
/// when that is turned on. A print that stopped part-way counts its share.
async fn account(state: &Shared, job: &Job) {
    let (Some(slice), Some(print)) = (&job.slice, &job.print) else { return };
    let share = match print.outcome {
        Some(PrintOutcome::Finished) => 1.0,
        Some(PrintOutcome::Failed | PrintOutcome::Cancelled) => print.progress.unwrap_or(0).clamp(0, 100) as f64 / 100.0,
        _ => return,
    };
    let use_g = crate::spoolman::use_per_tray(&slice.filaments, &print.ams_mapping, share);
    let spools = crate::spoolman::for_printer(state, &print.printer_id).await;
    let prices = crate::spoolman::prices_for_printer(state, &print.printer_id).await;
    let hours = print.ended_at.map(|e| (e - print.started_at).max(0) as f64 / 3600.0);
    let costs = state.store.read(|d| d.cost.clone()).await;
    let cost = crate::spoolman::cost(&use_g, &prices, hours, &costs, true);
    let settings = state.store.read(|d| d.spoolman.clone()).await;
    let booked = if settings.book_usage && !spools.is_empty() && share > 0.0 {
        crate::spoolman::book(state, &settings, &use_g, &spools).await
    } else {
        Vec::new()
    };
    let _ = crate::jobs::update_job(state, &job.id, |j| {
        if let Some(c) = &cost {
            j.push_log(format!("Cost: {:.2} {}", c.total, c.currency).trim_end().to_string());
        }
        j.cost = cost.clone();
        if !booked.is_empty() {
            j.push_log(format!("Booked in Spoolman: {}", booked.join(", ")));
        }
    })
    .await;
}
