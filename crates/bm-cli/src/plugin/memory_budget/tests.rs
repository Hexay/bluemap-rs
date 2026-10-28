use super::*;

fn budget(container_mib: u64, heap_mib: u64, overhead_mib: u64) -> MemoryBudget {
    let budget_mib = container_mib as i64 - heap_mib as i64 - overhead_mib as i64;
    MemoryBudget { container_mib, heap_mib, overhead_mib, budget_mib }
}

fn run(core_limit: Option<u64>, auto: bool, b: Option<MemoryBudget>) -> (Option<u64>, Option<(Level, String)>) {
    let mut core = CoreConfig { memory_limit: core_limit, ..CoreConfig::default() };
    let plugin = PluginConfig { auto_memory_limit: auto, ..PluginConfig::default() };
    let line = apply_container_budget(&mut core, &plugin, b);
    (core.memory_limit, line)
}

#[test]
fn floor_is_one_render_thread() {
    assert_eq!(FLOOR_MIB, 184);
}

#[test]
fn no_container_limit_changes_nothing() {
    assert_eq!(run(None, true, None), (None, None));
    assert_eq!(run(Some(1 << 30), true, None), (Some(1 << 30), None));
}

#[test]
fn budget_becomes_the_limit() {
    let (limit, line) = run(None, true, Some(budget(4096, 3072, 563)));
    assert_eq!(limit, Some(461 << 20));
    let (level, line) = line.unwrap();
    assert_eq!(level, Level::Info);
    assert_eq!(
        line,
        "Container memory limit 4096 MiB, JVM max heap 3072 MiB (+ ~563 MiB non-heap): memory-limit for the core is \
         461 MiB (override with memory-limit in core.conf)"
    );
}

#[test]
fn core_conf_wins() {
    let (limit, line) = run(Some(1 << 30), true, Some(budget(4096, 3891, 645)));
    assert_eq!(limit, Some(1 << 30));
    let (level, line) = line.unwrap();
    assert_eq!(level, Level::Info);
    assert!(line.ends_with("using the memory-limit of 1024 MiB from core.conf (automatic: 0 MiB)"), "{line}");
}

#[test]
fn plugin_conf_turns_it_off() {
    let (limit, line) = run(None, false, Some(budget(4096, 3072, 563)));
    assert_eq!(limit, None);
    assert!(line.unwrap().1.contains("auto-memory-limit is off"));
}

#[test]
fn too_small_starts_at_the_floor_and_says_what_to_change() {
    // Pterodactyl's egg: -XX:MaxRAMPercentage=95.0 in a 4 GiB container
    let (limit, line) = run(None, true, Some(budget(4096, 3891, 645)));
    assert_eq!(limit, Some(FLOOR_MIB << 20));
    let (level, line) = line.unwrap();
    assert_eq!(level, Level::Warning);
    // deficit 184 + 440 = 624: heap 3891 - 624 = 3267 = 79 % of 4096
    assert!(line.contains("leaves the BlueMap core -440 MiB but it needs 184 MiB"), "{line}");
    assert!(
        line.ends_with(
            "lower -XX:MaxRAMPercentage to 79 (or -Xmx to 3267M), or raise the server's memory limit to at least \
             4720 MiB"
        ),
        "{line}"
    );
}

#[test]
fn tiny_container_only_suggests_raising_it() {
    let (limit, line) = run(None, true, Some(budget(512, 256, 281)));
    assert_eq!(limit, Some(FLOOR_MIB << 20));
    let line = line.unwrap().1;
    assert!(line.ends_with("to fix it, raise the server's memory limit to at least 721 MiB"), "{line}");
}

#[test]
fn exactly_the_floor_is_enough() {
    let (limit, line) = run(None, true, Some(budget(1000, 500, 316)));
    assert_eq!(limit, Some(184 << 20));
    assert_eq!(line.unwrap().0, Level::Info);
}
