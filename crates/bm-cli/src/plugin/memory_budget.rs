//! The shim's container memory budget (`Hello.memoryBudget`) as this run's `memory-limit` when core.conf sets none
//! (docs/15 §2).

use bm_config::{CoreConfig, PluginConfig};
use bm_ipc::MemoryBudget;

use crate::log::Level;
use crate::throttle::memory::{BASE_BYTES, PER_THREAD_BYTES, mib};

/// The core's estimated peak at one render thread; a smaller budget is raised to it.
pub const FLOOR_MIB: u64 = mib(BASE_BYTES + PER_THREAD_BYTES);

/// Sets `core.memory_limit` from `budget` unless core.conf sets one or plugin.conf turns this off; returns the line
/// to log (`None` without a container limit).
pub fn apply_container_budget(
    core: &mut CoreConfig,
    plugin: &PluginConfig,
    budget: Option<MemoryBudget>,
) -> Option<(Level, String)> {
    let b = budget?;
    let detected = format!("Container memory limit {} MiB, JVM max heap {} MiB", b.container_mib, b.heap_mib);
    if let Some(user) = core.memory_limit {
        return Some((
            Level::Info,
            format!(
                "{detected}: using the memory-limit of {} MiB from core.conf (automatic: {} MiB)",
                mib(user),
                b.budget_mib.max(0)
            ),
        ));
    }
    if !plugin.auto_memory_limit {
        return Some((Level::Info, format!("{detected}: auto-memory-limit is off in plugin.conf, no memory-limit")));
    }
    let deficit = FLOOR_MIB as i64 - b.budget_mib;
    if deficit <= 0 {
        core.memory_limit = Some((b.budget_mib as u64) << 20);
        return Some((
            Level::Info,
            format!(
                "{detected} (+ ~{} MiB non-heap): memory-limit for the core is {} MiB (override with memory-limit in \
                 core.conf)",
                b.overhead_mib, b.budget_mib
            ),
        ));
    }
    core.memory_limit = Some(FLOOR_MIB << 20);
    Some((Level::Warning, too_small(&b, deficit as u64)))
}

/// Lowering the heap by the deficit is enough: the JVM's non-heap estimate shrinks with it.
fn too_small(b: &MemoryBudget, deficit: u64) -> String {
    let fix_container = format!("raise the server's memory limit to at least {} MiB", b.container_mib + deficit);
    let target_heap = b.heap_mib.saturating_sub(deficit);
    let percent = target_heap * 100 / b.container_mib.max(1);
    let fix = if target_heap >= 256 && percent >= 1 {
        format!("lower -XX:MaxRAMPercentage to {percent} (or -Xmx to {target_heap}M), or {fix_container}")
    } else {
        fix_container
    };
    format!(
        "Container memory limit {} MiB, JVM max heap {} MiB (+ ~{} MiB non-heap): that leaves the BlueMap core {} MiB \
         but it needs {FLOOR_MIB} MiB, and the host may kill the whole server when it runs out. Rendering with the \
         minimum (memory-limit {FLOOR_MIB} MiB); to fix it, {fix}",
        b.container_mib, b.heap_mib, b.overhead_mib, b.budget_mib
    )
}

#[cfg(test)]
mod tests;
