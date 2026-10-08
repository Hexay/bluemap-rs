//! Read-only commands: status, version, help, maps, tasks.

use std::sync::PoisonError;

use bm_engine::PauseReason;
use serde_json::Value;

use super::super::core::{CORE_VERSION, Core};
use super::super::session::Session;
use super::super::text::{self, BASE, FROZEN, HIGHLIGHT, INFO, POSITIVE};
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

pub fn status(core: &Core, s: &Session, say: Say) -> i32 {
    let mut body = Vec::new();
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    if !enabled {
        body.push(text::format("❌ render-threads are %", &["stopped"]));
        body.push(text::format("use % to start rendering", &["/bluemap start"]));
    } else if s.queue.is_paused() {
        body.push(text::format("⌛ render-threads are %", &["paused"]));
        body.extend(pause_lines(core, s));
    } else if let Some((run, progress)) = s.queue.current_run() {
        body.extend(active_task(s, run, progress));
    } else {
        body.push(text::format("✔ render-threads are %", &["idle"]));
    }
    let pending = s.queue.pending();
    if pending > 0 {
        body.push(text::format("% pending tasks", &[&pending.to_string()]));
    }
    let rss = bm_ipc::resident_memory().map_or("?".into(), |b| format!("{} MiB", b >> 20));
    body.push(text::format("core memory: %", &[&rss]));
    say(text::paragraph("Status", body));
    1
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

pub fn version(core: &Core, say: Say) -> i32 {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).to_string();
    let rss = bm_ipc::resident_memory().map_or("?".into(), |b| format!("{} MiB (core RSS)", b >> 20));
    let version = format!("{} (bluemap-rs {CORE_VERSION})", bm_engine::BLUEMAP_VERSION);
    say(text::paragraph(
        "Version",
        vec![
            text::format("Version: %", &[&version]),
            text::format("Implementation: %", &[&core.hello.platform]),
            text::format("Minecraft: %", &[&core.hello.mc_version]),
            text::format("Available Processors: %", &[&cores]),
            text::format("Available Memory: %", &[&rss]),
        ],
    ));
    1
}

pub fn help(say: Say) -> i32 {
    let mut body: Vec<_> = super::parse::SPEC
        .commands
        .iter()
        .map(|u| vec![text::span(format!("/bluemap {}", u.usage).trim_end().to_owned(), HIGHLIGHT)])
        .collect();
    body.dedup();
    body.push(text::format("Wiki: %", &["https://bluemap.bluecolored.de/wiki/"]));
    body.push(text::format("Discord: %", &["https://discord.gg/zmkyJa3"]));
    say(text::paragraph("Help", body));
    1
}

pub fn maps(s: &Session, say: Say) -> i32 {
    let mut maps = s.maps.all();
    maps.sort_by_key(|m| m.config.sorting);
    let state = s.state.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let current = s.queue.current_task();
    let pending = s.queue.pending_tasks();
    let body = maps
        .iter()
        .map(|m| {
            let name = m.config.name.clone().unwrap_or_else(|| m.id.clone());
            let label = format!("{name} ({})", m.id);
            let n = pending.iter().filter(|t| t.map == m.id).count();
            let (icon, color, detail) = if current.as_ref().is_some_and(|t| t.map == m.id) {
                ("⛏", INFO, "is currently being updated".to_owned())
            } else if n > 0 {
                ("⌛", INFO, format!("has {n} pending task{}", if n == 1 { "" } else { "s" }))
            } else if state.is_frozen(&m.id) {
                ("❄", FROZEN, "is frozen".to_owned())
            } else {
                ("✔", POSITIVE, String::new())
            };
            let mut line = vec![text::span(format!("{icon} "), color), text::span(label, HIGHLIGHT)];
            if !detail.is_empty() {
                line.push(text::span(format!(" {detail}"), BASE));
            }
            line
        })
        .collect();
    say(text::paragraph("Maps", body));
    1
}

pub fn tasks(s: &Session, say: Say) -> i32 {
    let current = s.queue.current();
    let pending = s.queue.pending_tasks();
    if current.is_none() && pending.is_empty() {
        say(text::paragraph("Tasks", vec![vec![text::span("There are no scheduled tasks", BASE)]]));
        return 1;
    }
    let mut body = Vec::new();
    if let (Some((desc, progress)), Some(task)) = (current, s.queue.current_task()) {
        let pct = format!("{:.3}%", progress * 100.0);
        body.push(text::format("⛏ [%] % %", &[&task_ref(&task), &desc, &pct]));
    }
    for task in pending.iter().take(10) {
        body.push(text::format("⌛ [%] %", &[&task_ref(task), &task.description()]));
    }
    if pending.len() > 10 {
        body.push(text::format("... % more scheduled tasks ...", &[&(pending.len() - 10).to_string()]));
    }
    say(text::paragraph("Tasks", body));
    1
}
