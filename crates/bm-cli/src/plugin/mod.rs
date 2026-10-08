//! `bluemap --plugin-ipc`: the core of a server plugin (docs/13), driven by the JVM shim over stdin/stdout with
//! the `bm-ipc` protocol. It does what upstream's `Plugin` does on the server; the shim supplies worlds, players,
//! markers, commands and API calls.

mod blockstates;
mod commands;
mod core;
mod live;
mod ops;
mod outbox;
mod rpc;
mod rstate;
mod session;
mod state;
mod tasks_dat;
mod text;
mod timers;
mod watchers;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bm_ipc::{CoreLock, CoreMsg, Frame, LogLevel, PROTOCOL, ShimMsg, read_frame};

use self::core::{CORE_VERSION, Core};
use self::outbox::Outbox;
use crate::log;

/// What the shim told us in `Hello`.
pub struct Hello {
    pub platform: String,
    pub mc_version: String,
    pub config_folder: PathBuf,
    pub mods_folder: Option<PathBuf>,
    pub max_memory_mib: Option<u64>,
    /// `defaultBlockstates` dump (JSON object), possibly empty.
    pub blockstates: Vec<u8>,
}

enum Event {
    Frame(Frame),
    /// stdin closed or unreadable: the shim is gone or asked us to stop.
    Eof,
    ParentGone,
}

/// The global rayon pool can be built once; later loads keep the first thread count. Always low OS priority.
pub fn init_render_pool(core: &bm_config::CoreConfig) {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = crate::throttle::build_render_pool(core, true);
    });
}

pub fn run(parent_pid: Option<u32>) -> ExitCode {
    bm_ipc::ignore_interrupts();
    let out = match bm_ipc::take_stdout() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("bluemap: can't take stdout for IPC: {e}");
            return ExitCode::from(1);
        }
    };
    let (tx, rx) = mpsc::channel();
    let reader_tx = tx.clone();
    std::thread::Builder::new()
        .name("bluemap-ipc-in".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                match read_frame(&mut stdin) {
                    Ok(Some(f)) => {
                        if reader_tx.send(Event::Frame(f)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("bluemap: ipc read failed: {e}");
                        break;
                    }
                }
            }
            let _ = reader_tx.send(Event::Eof);
        })
        .expect("spawn ipc reader");
    if let Some(pid) = parent_pid {
        std::thread::Builder::new()
            .name("bluemap-parent-watchdog".into())
            .spawn(move || {
                bm_ipc::wait_for_parent_exit(pid);
                let _ = tx.send(Event::ParentGone);
            })
            .expect("spawn watchdog");
    }

    let outbox = Outbox::start(out);
    let Some((hello, worlds)) = handshake(&rx, &outbox) else {
        outbox.close();
        return ExitCode::from(3);
    };
    let lock = match CoreLock::acquire(&hello.config_folder) {
        Ok(lock) => lock,
        Err(e) => {
            outbox.log(LogLevel::Error, &format!("{e}"));
            outbox.close();
            return ExitCode::from(4);
        }
    };
    let core = Arc::new(Core::new(outbox, hello, worlds));
    {
        let core = core.clone();
        log::set_sink(move |level, msg| {
            let level = match level {
                log::Level::Info => LogLevel::Info,
                log::Level::Warning => LogLevel::Warning,
                log::Level::Error => LogLevel::Error,
            };
            core.out.log(level, msg);
        });
    }
    let stop = Arc::new(AtomicBool::new(false));
    let timer = timers::spawn(core.clone(), stop.clone());
    {
        let core = core.clone();
        std::thread::Builder::new().name("BlueMap-Load".into()).spawn(move || core.load()).expect("spawn loader");
    }
    event_loop(&core, &rx);

    log::info("Stopping...");
    stop.store(true, Ordering::SeqCst);
    let _ = timer.join();
    core.unload(false);
    log::info("Saved and stopped!");
    core.out.close();
    drop(lock);
    ExitCode::SUCCESS
}

/// Waits for `Hello`, answers `Welcome` or `Incompatible`.
fn handshake(rx: &Receiver<Event>, out: &Outbox) -> Option<(Hello, Vec<bm_ipc::WorldInfo>)> {
    let Ok(Event::Frame(frame)) = rx.recv() else { return None };
    let hello = frame.parse::<ShimMsg>();
    let Ok(ShimMsg::Hello {
        protocol, platform, mc_version, config_folder, mods_folder, max_memory_mib, worlds, ..
    }) = hello
    else {
        eprintln!("bluemap: expected Hello, got {}", frame.kind());
        return None;
    };
    if protocol != PROTOCOL {
        out.send(CoreMsg::Incompatible { protocol: PROTOCOL, core_version: CORE_VERSION.to_owned() });
        return None;
    }
    out.send(CoreMsg::Welcome {
        protocol: PROTOCOL,
        core_version: CORE_VERSION.to_owned(),
        compat_version: bm_engine::BLUEMAP_VERSION.to_owned(),
        pid: std::process::id(),
    });
    let hello = Hello {
        platform,
        mc_version,
        config_folder: PathBuf::from(config_folder),
        mods_folder: mods_folder.map(PathBuf::from),
        max_memory_mib,
        blockstates: frame.body,
    };
    Some((hello, worlds))
}

/// Dispatches shim messages until `Shutdown`, EOF or the parent's death. Slow work runs on its own threads.
fn event_loop(core: &Arc<Core>, rx: &Receiver<Event>) {
    for event in rx {
        let frame = match event {
            Event::Frame(f) => f,
            Event::Eof | Event::ParentGone => return,
        };
        let msg = match frame.parse::<ShimMsg>() {
            Ok(m) => m,
            Err(e) => {
                log::warn(&format!("Ignoring IPC message '{}': {e}", frame.kind()));
                continue;
            }
        };
        match msg {
            ShimMsg::Shutdown => return,
            ShimMsg::Reply(reply) => core.out.on_reply(reply),
            ShimMsg::Players { players } => core.live.set_players(players, core.session().as_deref()),
            ShimMsg::PlayerJoin { .. } | ShimMsg::PlayerLeave { .. } => {
                *core.limit_check_at.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(Instant::now() + Duration::from_secs(1));
            }
            ShimMsg::Markers { map, more } => core.live.on_markers(&map, more, frame.body, core.session().as_deref()),
            ShimMsg::WorldAdded { world } => {
                let mut worlds = core.worlds.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                worlds.retain(|w| w.id != world.id);
                worlds.push(world);
            }
            ShimMsg::WorldRemoved { id } => {
                core.worlds.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|w| w.id != id)
            }
            ShimMsg::ServerLoad { mspt } => ops::on_server_load(core, mspt),
            ShimMsg::Hello { .. } => log::warn("Ignoring a second Hello"),
            ShimMsg::Command { id, input, sender } => {
                let core = core.clone();
                spawn("bluemap-command", move || commands::execute(&core, id, &input, &sender));
            }
            ShimMsg::Reload { id, light } => {
                let core = core.clone();
                spawn("bluemap-reload", move || {
                    core.reload(light);
                    core.out.send(CoreMsg::Reply(bm_ipc::Reply::ok(id, serde_json::Value::Null)));
                });
            }
            rpc_msg => {
                let core = core.clone();
                spawn("bluemap-rpc", move || rpc::handle(&core, rpc_msg, frame.body));
            }
        }
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error(&format!("Failed to start a {name} thread: {e}"));
    }
}
