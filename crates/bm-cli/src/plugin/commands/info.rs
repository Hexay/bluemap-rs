//! Read-only commands: status, version, help, maps, tasks, storages.

use std::sync::PoisonError;

use super::super::core::{CORE_VERSION, Core};
use super::super::session::Session;
use super::super::text::{self, BASE, FROZEN, HIGHLIGHT, INFO, POSITIVE};
use super::{Say, task_ref};

pub fn status(s: &Session, say: Say) -> i32 {
    let mut body = Vec::new();
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    if !enabled {
        body.push(text::format("❌ render-threads are %", &["stopped"]));
        body.push(text::format("use % to start rendering", &["/bluemap start"]));
    } else if s.queue.is_paused() {
        body.push(text::format("⌛ render-threads are %", &["paused"]));
        let limit = s.service.config.plugin.player_render_limit.to_string();
        body.push(text::format("there are % or more players online", &[&limit]));
    } else if let Some((desc, progress)) = s.queue.current() {
        body.push(text::format("⛏ currently running: %", &[&desc]));
        body.push(text::format("progress: %", &[&format!("{:.3}%", progress * 100.0)]));
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

pub fn storages(s: &Session, say: Say) -> i32 {
    let body = s.service.config.storages.keys().map(|id| vec![text::span(id.clone(), HIGHLIGHT)]).collect();
    say(text::paragraph("Storages", body));
    1
}
