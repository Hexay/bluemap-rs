//! `/bluemap …` execution (`CommandExecutor` + `commands/commands/*`): parse against `commands.json`, check the
//! sender's permission node and the loaded state, run, and stream text components back.

mod actions;
mod info;
pub mod parse;

use bm_ipc::{CommandSender, CoreMsg};
use serde_json::Value;

use super::core::Core;
use super::session::Session;
use super::text::{self, NEGATIVE};
use parse::{Ids, Matched};

/// Sends one message to the command's sender.
pub type Say<'a> = &'a mut dyn FnMut(Value);

pub fn execute(core: &Core, id: u64, input: &str, sender: &CommandSender) {
    let mut say = |component| core.out.send(CoreMsg::CommandOutput { id, component });
    let result = run(core, input, sender, &mut say);
    core.out.send(CoreMsg::CommandDone { id, result });
}

fn run(core: &Core, input: &str, sender: &CommandSender, say: Say) -> i32 {
    let session = core.session();
    let (maps, storages) = match &session {
        Some(s) => {
            (s.maps.all().iter().map(|m| m.id.clone()).collect(), s.service.config.storages.keys().cloned().collect())
        }
        None => (Vec::new(), Vec::new()),
    };
    let Some(m) = parse::parse(input.trim().trim_start_matches('/'), &Ids { maps: &maps, storages: &storages }) else {
        say(text::one("Unknown or incomplete command!", NEGATIVE));
        return 0;
    };
    if !sender.permissions.iter().any(|p| p == m.permission) {
        say(text::one("You don't have permission to use this command!", NEGATIVE));
        return 0;
    }
    if matches!(m.usage, "reload" | "reload light") {
        return actions::reload(core, m.usage == "reload light", say);
    }
    let Some(s) = session.filter(|s| s.is_loaded() && !core.is_loading()) else {
        say(not_loaded(core));
        return 0;
    };
    dispatch(core, &s, &m, sender, say)
}

fn dispatch(core: &Core, s: &Session, m: &Matched, sender: &CommandSender, say: Say) -> i32 {
    let usage = m.usage;
    let first = usage.split_whitespace().next().unwrap_or("");
    match first {
        "" => info::status(s, say),
        "version" => info::version(core, say),
        "help" => info::help(say),
        "maps" => info::maps(s, say),
        "tasks" if usage == "tasks" => info::tasks(s, say),
        "tasks" => actions::cancel_tasks(s, m.get("task-ref"), say),
        "storages" if usage == "storages" => info::storages(s, say),
        "start" | "stop" => actions::start_stop(core, s, first == "start", say),
        "freeze" | "unfreeze" => actions::freeze(core, s, m.get("map").unwrap_or_default(), first == "freeze", say),
        "purge" => actions::purge(s, m.get("map").unwrap_or_default(), say),
        "update" | "fix-edges" | "force-update" => actions::update(core, s, m, sender, say),
        _ => {
            say(text::one("This command is not supported by bluemap-rs yet.", NEGATIVE));
            0
        }
    }
}

/// `Commands.checkPluginLoaded`.
fn not_loaded(core: &Core) -> Value {
    if core.is_loading() {
        text::lines(vec![
            vec![text::span("⌛ BlueMap is still loading!", text::INFO)],
            vec![text::span("Please try again in a few seconds.", text::BASE)],
        ])
    } else {
        text::lines(vec![
            vec![text::span("❌ BlueMap is not loaded!", NEGATIVE)],
            text::format("Check your server-console for errors or warnings and try using %.", &["/bluemap reload"]),
        ])
    }
}

/// A stable 4-hex reference for a queued task (`Commands.getRefForTask`).
pub fn task_ref(task: &bm_engine::RenderTask) -> String {
    let desc = format!("{}|{:?}|{}", task.map, task.strategy, task.description());
    format!("{:04x}", bm_java::string_hash(&desc) as u32 & 0xffff)
}
