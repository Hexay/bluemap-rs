//! Beyond-parity render throttling (docs/15): render-pool sizing and OS priority, the `memory-limit` guard and the
//! server-load (MSPT) monitor. Each pauses the render queue under its own [`bm_engine::PauseReason`].

pub mod load;
pub mod memory;

use std::sync::atomic::{AtomicBool, Ordering};

use bm_config::CoreConfig;

use crate::log;

/// Java's `Thread.NORM_PRIORITY`; `render-thread-priority` below it lowers the OS priority, above it changes nothing.
const NORM_PRIORITY: i32 = 5;

/// Builds the global render pool: the configured thread count lowered to fit `memory-limit`, at low OS priority
/// when `low_priority` (plugin) or `render-thread-priority` < 5. Returns the thread count.
pub fn build_render_pool(core: &CoreConfig, low_priority: bool) -> Result<usize, rayon::ThreadPoolBuildError> {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let resolved = core.resolve_render_thread_count(cores);
    let threads = memory::fit_threads(resolved, core.memory_limit);
    if threads < resolved {
        log::info(&format!(
            "Using {threads} instead of {resolved} render threads to stay within the memory-limit of {} MiB",
            core.memory_limit.map_or(0, memory::mib)
        ));
    }
    let low = low_priority || core.render_thread_priority < NORM_PRIORITY;
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("bluemap-render-{i}"))
        .start_handler(move |_| {
            if low {
                lower_priority();
            }
        })
        .build_global()?;
    Ok(threads)
}

fn lower_priority() {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if let Err(e) = bm_ipc::lower_thread_priority()
        && !WARNED.swap(true, Ordering::Relaxed)
    {
        log::warn(&format!("Could not lower the render threads' priority: {e}"));
    }
}
