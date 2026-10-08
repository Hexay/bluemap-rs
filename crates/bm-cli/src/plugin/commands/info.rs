//! Read-only commands: status, version, help, maps, tasks.

use std::collections::HashSet;
use std::sync::PoisonError;

use bm_engine::PauseReason;
use serde_json::Value;

use super::super::core::{CORE_VERSION, Core};
use super::super::session::Session;
use super::super::text::{self, BASE, FROZEN, HIGHLIGHT, INFO, NEGATIVE, POSITIVE};
use super::{Say, task_ref};
use crate::eta;
use crate::throttle::memory;

/// Why the render threads are paused, one line per reason other than `Stopped`.
pub fn pause_lines(core: &Core, s: &Session) -> Vec<Vec<Value>> {
    let lines = s.queue.pause_reasons().iter().filter_map(|reason| match reason {
        PauseReason::Stopped => None,
        PauseReason::PlayerLimit => {
            let limit = s.service.config.plugin.player_render_limit.to_string();
            Some(text::format("there are % or more players online", &[&limit]))
        }
        PauseReason::Memory => {
            let limit = s.service.config.core.memory_limit.map_or(0, memory::mib);
            Some(text::format("core memory is above the memory-limit of %", &[&format!("{limit} MiB")]))
        }
        PauseReason::ServerLoad => {
            let mspt = core.load.lock().unwrap_or_else(PoisonError::into_inner).average().unwrap_or_default();
            Some(text::format("server is lagging (MSPT %)", &[&format!("{mspt:.1}")]))
        }
    });
    lines.collect()
}

/// `StatusCommand.Status`: render-thread state, the running task, then a summary of the maps.
pub fn status(core: &Core, s: &Session, say: Say) -> i32 {
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    let paused = s.queue.is_paused();
    let running = s.queue.current_run().filter(|_| enabled && !paused);
    let mut body = render_threads(core, s, enabled, paused, running.is_some());
    if let Some((run, progress)) = running {
        body.extend(active_task(s, run, progress));
    }
    body.extend(map_summary(s, running.is_some()));
    say(text::paragraph("Status", body));
    1
}

fn with_details(line: Vec<Vec<Value>>, details: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    let mut out = line;
    out.extend(text::details(details.into_iter().map(|d| vec![d]).collect(), BASE));
    out
}

fn render_threads(core: &Core, s: &Session, enabled: bool, paused: bool, processing: bool) -> Vec<Vec<Value>> {
    if !enabled {
        let line = text::fill("❌ render-threads are %", &[text::hl("stopped")], NEGATIVE);
        return with_details(line, vec![text::format("use % to start rendering", &["/bluemap start"])]);
    }
    if paused {
        return with_details(text::fill("⌛ render-threads are %", &[text::hl("paused")], INFO), pause_lines(core, s));
    }
    let threads = rayon::current_num_threads();
    let template = if threads == 1 { "✔ % render-thread is %" } else { "✔ % render-threads are %" };
    let state = if processing { "running" } else { "idle" };
    let line = text::fill(template, &[text::hl(&threads.to_string()), text::hl(state)], POSITIVE);
    match s.queue.last_busy().filter(|_| !processing) {
        Some(at) => {
            let ago = text::duration(at.elapsed().as_millis() as i64);
            with_details(line, vec![text::format("last active % ago", &[&ago])])
        }
        None => line,
    }
}

/// `StatusCommand.mapSummary`: maps with queued tasks, up to date, frozen; the running task's map excluded.
fn map_summary(s: &Session, exclude_running: bool) -> Vec<Vec<Value>> {
    let state = s.state.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let current = s.queue.current_task().map(|t| t.map);
    let queued: HashSet<String> =
        current.iter().cloned().chain(s.queue.pending_tasks().into_iter().map(|t| t.map)).collect();
    let (mut pending, mut updated, mut frozen) = (Vec::new(), Vec::new(), Vec::new());
    for m in s.maps.all() {
        if exclude_running && current.as_ref() == Some(&m.id) {
            continue;
        }
        let set = if queued.contains(&m.id) {
            &mut pending
        } else if state.is_frozen(&m.id) {
            &mut frozen
        } else {
            &mut updated
        };
        set.push(m.id.clone());
    }
    let summary = |maps: &[String], single: &str, multiple: &str, color: &str| match maps {
        [] => Vec::new(),
        [one] => text::fill(single, &[text::hl(one)], color),
        _ => text::fill(multiple, &[text::hl(&maps.len().to_string())], color),
    };
    let mut out = summary(&pending, "⌛ map % has pending updates", "⌛ % maps have pending updates", INFO);
    out.extend(summary(&updated, "✔ map % is updated", "✔ % maps are updated", POSITIVE));
    out.extend(summary(&frozen, "❄ map % is frozen", "❄ % maps are frozen", FROZEN));
    out
}

/// `StatusCommand.activeTask`: what runs, its progress and remaining time, between empty lines.
fn active_task(s: &Session, run: u64, progress: f64) -> Vec<Vec<Value>> {
    let mut info = match s.queue.current_task() {
        Some(task) => text::fill("⛏ map % is currently being updated", &[text::hl(&task.map)], INFO),
        None => {
            let desc = s.queue.current().map(|(d, _)| d).unwrap_or_default();
            text::fill("⛏ currently running: %", &[text::hl(&desc)], INFO)
        }
    };
    let remaining = s.progress.lock().unwrap_or_else(PoisonError::into_inner).remaining_of((run, progress));
    let mut items = vec![vec![text::format("progress: %", &[&format!("{:.3}%", progress * 100.0)])]];
    if let Some(eta) = eta::status_remaining(remaining, progress) {
        items.push(vec![text::format("remaining time: %", &[&eta])]);
    }
    info.insert(0, Vec::new());
    info.extend(text::details(items, BASE));
    info.push(Vec::new());
    info
}

/// `VersionCommand`; our build stands where upstream prints its git hash. Memory is the server JVM's max heap.
pub fn version(core: &Core, say: Say) -> i32 {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).to_string();
    let gib =
        core.hello.max_memory_mib.map_or("?".into(), |mib| format!("{:?} GiB", (mib as f64 / 102.4).round() / 10.0));
    let mut body = text::fill("Version: %", &[(bm_engine::BLUEMAP_VERSION, INFO)], BASE);
    let details = vec![
        vec![text::span(format!("bluemap-rs {CORE_VERSION}"), BASE)],
        text::format("Implementation: %", &[&core.hello.platform]),
        text::format("Minecraft: %", &[&core.hello.mc_version]),
    ];
    body = with_details(body, details);
    body.push(text::format("Available Processors: %", &[&cores]));
    body.push(text::format("Available Memory: %", &[&gib]));
    say(text::paragraph("Version", body));
    1
}

pub fn help(say: Say) -> i32 {
    let body = vec![
        text::format("Wiki: %", &["https://bluemap.bluecolored.de/wiki/"]),
        text::format("Discord: %", &["https://discord.gg/zmkyJa3"]),
    ];
    say(text::paragraph("Help", body));
    1
}

pub fn maps(s: &Session, say: Say) -> i32 {
    let mut maps = s.maps.all();
    maps.sort_by_key(|m| m.config.sorting);
    let state = s.state.lock().unwrap_or_else(PoisonError::into_inner).clone();
    // upstream's RenderManager queue: its head is the "current" task even while render-threads are stopped
    let current = s.queue.current().map(|(_, progress)| (s.queue.current_task().map(|t| t.map), progress));
    let progress = current.as_ref().map_or(0.0, |c| c.1);
    // map of each scheduled task, None for a task without one
    let scheduled: Vec<Option<String>> =
        current.map(|c| c.0).into_iter().chain(s.queue.pending_tasks().into_iter().map(|t| Some(t.map))).collect();
    let mut body = Vec::new();
    for m in &maps {
        let (mut icon, mut color, mut details) = ("✔", POSITIVE, Vec::new());
        if state.is_frozen(&m.id) {
            (icon, color) = ("❄", FROZEN);
            details.push(text::format("is %", &["frozen"]));
        }
        let pending = scheduled.iter().skip(1).filter(|t| t.as_ref() == Some(&m.id)).count();
        if pending > 0 {
            (icon, color) = ("⌛", INFO);
            let template = if pending == 1 { "has % pending task" } else { "has % pending tasks" };
            details.insert(0, text::format(template, &[&pending.to_string()]));
        }
        if scheduled.first().is_some_and(|t| t.as_ref() == Some(&m.id)) {
            (icon, color) = ("⛏", INFO);
            details.insert(0, text::format("is currently being updated: %", &[&format!("{:.3}%", progress * 100.0)]));
        }
        let line = vec![text::span(format!("{icon} "), color), text::span(m.id.clone(), HIGHLIGHT)];
        body.extend(with_details(vec![line], details));
    }
    say(text::paragraph("Maps", body));
    1
}

/// `TasksCommand.taskList`: the last 3 finished tasks, the running one, then up to 6 queued.
pub fn tasks(s: &Session, say: Say) -> i32 {
    let completed = s.queue.completed();
    let mut pending: Vec<(String, String)> =
        s.queue.pending_tasks().iter().map(|t| (task_ref(t), t.description())).collect();
    // upstream's queue head is the "current" task, running or not
    let current = match (s.queue.current(), s.queue.current_task()) {
        (Some(_), Some(task)) => Some((task_ref(&task), task.description())),
        (Some((desc, _)), None) => Some((String::new(), desc)),
        (None, _) if !pending.is_empty() => Some(pending.remove(0)),
        (None, _) => None,
    };
    let mut body = Vec::new();
    if completed.len() > 3 {
        body.push(vec![text::span("... more done tasks ...", BASE)]);
    }
    for desc in &completed[completed.len().saturating_sub(3)..] {
        body.push(vec![
            text::span("✔", POSITIVE),
            text::span(" ", BASE),
            serde_json::json!({"text": "(done)", "color": BASE, "italic": true}),
            text::span(format!(" {desc}"), BASE),
        ]);
    }
    if let Some((r, desc)) = &current {
        body.push(text::fill("% % % ...", &[("⛏", INFO), (&format!("[{r}]"), BASE), (desc, "")], HIGHLIGHT).remove(0));
    }
    for (r, desc) in pending.iter().take(6) {
        body.push(text::fill("% % %", &[("⌛", INFO), (&format!("[{r}]"), BASE), (desc, "")], INFO).remove(0));
    }
    if pending.len() > 6 {
        let more = (pending.len() - 6).to_string();
        body.push(text::fill("... % more scheduled tasks ...", &[(&more, "")], BASE).remove(0));
    }
    if current.is_none() && pending.is_empty() {
        body.push(text::fill("% no pending tasks, all done", &[("✔", POSITIVE)], POSITIVE).remove(0));
    }
    say(text::paragraph("Tasks", body));
    1
}
