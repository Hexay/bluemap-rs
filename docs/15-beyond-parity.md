# 15 — Beyond parity: render pausing, memory limit, CPU throttling

Upstream BlueMap's only render knobs are `render-thread-count`, `render-thread-priority` and `player-render-limit`
(docs/06). bluemap-rs adds three, all through hidden keys (absent from the generated templates, so existing configs
and Java BlueMap are unaffected).

## 1. Pause reasons

`RenderQueue` (`crates/bm-engine/src/queue.rs`, `queue/pause.rs`) is paused by a set of `PauseReason`s; it runs
only when none is set:

| Reason | Set by | Cleared by |
|---|---|---|
| `Stopped` | `/bluemap stop`, `RenderManager.stop()`, persisted `renderThreadsEnabled: false` | `/bluemap start`, `RenderManager.start()` |
| `PlayerLimit` | `player-render-limit` reached (checked 1 s after a join/leave) | fewer players, or `/bluemap start` until the next check (upstream) |
| `Memory` | core RSS above `memory-limit` (§2) | RSS below 90 % of it |
| `ServerLoad` | 10 s average MSPT above `render-pause-mspt` (§3) | average below `render-resume-mspt` |

- The first reason cancels the running task (it stops after its current region) and queues it again in front;
  later reasons only add to the set. A running job (`storages … delete`) finishes. Finished regions stay done, but
  a forced task (`-f`, `force-update`) starts over from its first region, as with upstream-style `/bluemap stop`.
- `take(exit_when_idle)` (CLI `-r`) no longer ends the run while the queue is paused with work left; it waits for
  the resume.
- `/bluemap start` clears `Stopped` and `PlayerLimit`, as upstream's restarts the threads. If `Memory` or
  `ServerLoad` remain it says so ("...but they stay paused:" plus one line per reason); with only those set it
  answers "Render-Threads are paused:" and the reasons.
- `/bluemap` status keeps upstream's stopped wording; when paused it lists each reason: "there are N or more players
  online", "core memory is above the memory-limit of N MiB", "server is lagging (MSPT x)". `troubleshoot` reports
  the first reason. `RenderStatus.running`, `StateInfo.renderThreadsRunning` = no reason set.

## 2. `memory-limit` (core.conf)

```hocon
memory-limit: "2G"    # 512M, 1.5GiB, 2GB, 1048576 (bytes); unset or 0 = no limit
```

Units as typesafe-config's `getBytes`: `K`/`k`/`Ki`/`KiB`/`kibibytes` = 1024, `kB`/`KB`/`kilobytes` = 1000, likewise
M, G, T, P, E; fractions allowed. Garbage is a config error naming the key (`MemorySizeError`,
`crates/bm-config/src/de/size.rs`).

**At load** the render pool (`crates/bm-cli/src/throttle/mod.rs`, CLI and plugin) gets
`min(render-thread-count, max(1, (limit − base) / per_thread))` threads and logs at INFO when that lowered it:
`Using 1 instead of 8 render threads to stay within the memory-limit of 100 MiB`. A limit below the base still gets
one thread; the runtime guard handles the rest.

**At runtime** a guard (`throttle/memory.rs`; CLI: a thread during `-r`/`-u`, plugin: the 1 Hz timer) samples RSS
every second:
- RSS > limit → `pause(Memory)`, WARNING `Core memory (102 MiB) is above the memory-limit (100 MiB): pausing
  rendering`. RSS < 90 % → resume, INFO. The webserver keeps serving.
- Paused, nothing running and still above the limit for 60 s → WARNING that the limit is below the core's idle
  footprint; it resumes and stays warn-only until the next start.
- A forced task already interrupted once by the guard is not interrupted again (it would restart from scratch
  forever); one WARNING says it is let finish.

**Measured** (Windows peak working set, release build of this branch, whose render path is b58745d's; fixture
`structures`, compat file storage, 2 runs each):

```
py -3 tools/bench.py mem-structures-t<N> -n 2 --clean <dir>/web --cwd <dir> -- \
    target/release/bluemap.exe -c config -v 26.3 -r -f
```
(`<dir>` = a copy of `work/bluemap/structures/config` + the client jar, paths rewritten as `tools/accept.py`'s
`prepare`; `render-thread-count: N`.)

| Threads | 1 | 2 | 4 | 8 |
|---|---|---|---|---|
| Peak RSS (MiB) | 120 | 132 | 163 | 215 |
| Wall (s) | 23.5 | 13.4 | 8.4 | 7.0 |

Least-squares fit: 106 MiB + 13.7 MiB/thread. The constants are deliberately higher, **base 160 MiB, 24 MiB per
thread** (`BASE_BYTES`, `PER_THREAD_BYTES`): larger worlds keep bigger caches, and glibc's per-thread malloc arenas
raise Linux RSS (the perf session measured ~440 MB at 12 threads with optimized storage on Linux glibc;
160 + 12 × 24 = 448).

**Checked** with `memory-limit: "100M"`, `render-thread-count: 8`, `-r -f` on `structures`: 1 thread used; pause at
102 MiB, resume at 66 MiB one second later, the forced task restarted, exceeded the limit again and was let finish;
the render completed (exit 0, 1153 tiles, same as without a limit) and `bm-golden diff-render` against an unlimited
render found all 16,467,480 hires faces identical.

## 3. CPU throttling (plugins)

**Low priority.** Plugin mode: render threads always run below normal OS priority. CLI: `render-thread-priority`
(Java 1–10, default 5) ≤ 4 lowers them the same way, ≥ 5 leaves them alone (never raised). Applied in rayon's
`start_handler` via `bm_ipc::lower_thread_priority()`: Linux nice +10 on the thread (capped at 19), macOS QoS class
utility, Windows `THREAD_PRIORITY_BELOW_NORMAL`. A failure is logged once at WARNING and rendering continues.

**Pause on MSPT.** IPC protocol 2 adds shim → core `ServerLoad{mspt}` (crate docs of `bm-ipc`): the shared
`ServerLoadReporter` (`platforms/common/.../shim/load/`) sends `Platform.averageTickMillis()` once a second,
latest-wins: Paper `Server.getAverageTickTime()`, Fabric `MinecraftServer.getAverageTickTimeNanos() / 1e6` (last 100
ticks, as `/tick query`), Folia nothing (no global tick). The core (`throttle/load.rs`, `plugin/ops.rs`) keeps a 10 s
rolling average (pausing needs ≥ 5 samples) with hysteresis:

```hocon
# plugin.conf
render-pause-mspt: 45     # pause while the 10 s average is above; <= 0 disables
render-resume-mspt: 40    # resume below; outside (0, pause] means = pause
```

Logs: once per core process `Rendering pauses while the server's average tick time is above 45 ms` (first sample);
WARNING `The server is lagging (MSPT 48.0, limit 45): pausing rendering`; INFO `The server recovered (MSPT 31.1):
resuming rendering`. The state lives on the core, not the session, so it survives `/bluemap reload` (a new
session starts paused while lagging).

**Tests**: `bm-engine` queue tests (independent reasons, idle exit while paused), `bm-cli` throttle unit tests
(fit, hysteresis, idle give-up, MSPT window), `bm-config` `hidden_throttle_keys` + size parser, `bm-ipc` framing of
`ServerLoad`, `crates/bm-cli/tests/plugin_ipc.rs::server_load_pauses_and_resumes` (scripted shim), and one check per
`tools/e2e_paper.py` / `tools/e2e_fabric.py` run that the core received `ServerLoad`.
