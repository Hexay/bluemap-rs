// Keeps run.mjs's Chrome from loading a shared box (docs/25 "Harness"): each browser runs in its own transient
// systemd user scope (CPU quota, memory cap without swap, task cap, nice, hard lifetime) and is torn down by stopping
// the scope, which kills every helper. Without a working `systemd-run --user`: a plain spawn with an RSS watch.
import { execFileSync, spawn } from "node:child_process";
import { readFileSync } from "node:fs";

// switches that cut background work without touching what the page renders (all present in Chrome 1248's binary)
export const QUIET_CHROME_FLAGS = ["--num-raster-threads=1", "--renderer-process-limit=1",
  "--disable-background-networking", "--disable-component-update", "--disable-extensions", "--disable-sync",
  "--disable-default-apps", "--disable-breakpad", "--disable-domain-reliability", "--metrics-recording-only",
  "--no-pings", "--mute-audio"];

const NO_BUS = "disabled:";
const read = (path) => readFileSync(path, "utf8");
const quiet = { stdio: ["ignore", "pipe", "pipe"], encoding: "utf8", timeout: 15000 };

export function boxState() {
  const psi = /some avg10=([\d.]+)/.exec(read("/proc/pressure/cpu"))?.[1];
  const avail = /MemAvailable:\s+(\d+)/.exec(read("/proc/meminfo"))?.[1];
  return { load1: Number(read("/proc/loadavg").split(" ")[0]), cpuPsiSome10: Number(psi),
    memAvailableMB: Math.round(Number(avail) / 1024) };
}

// throws when the box has no room for a browser capped at maxRssMB plus a 1 GB margin
export function preflight(maxRssMB) {
  const s = boxState();
  if (s.memAvailableMB <= maxRssMB + 1024)
    throw new Error(`MemAvailable ${s.memAvailableMB} MB <= --max-rss ${maxRssMB} MB + 1024: not starting Chrome`);
  return s;
}

// polls every 5 s until cpu PSI some avg10 <= psi or maxMs passed; false on giving up
export async function waitForCalm(psi, maxMs, log) {
  const until = Date.now() + maxMs;
  for (;;) {
    const now = boxState().cpuPsiSome10;
    if (now <= psi) return true;
    if (Date.now() >= until) return false;
    log(`cpu PSI some avg10 ${now} > --wait-load ${psi}: waiting`);
    await new Promise((r) => setTimeout(r, Math.min(5000, until - Date.now())));
  }
}

let scopeWorks = null;
function systemdScopeWorks() {
  if (scopeWorks === null) {
    try {
      execFileSync("systemd-run", ["--user", "--scope", "--quiet", "-p", "CPUQuota=100%", "--", "true"], quiet);
      scopeWorks = true;
    } catch {
      scopeWorks = false;
    }
  }
  return scopeWorks;
}

const live = new Set();
let launched = 0;

// Starts `cmd args` capped; returns { child, unit, stats(), stop() }. stats() reads the scope's cgroup (call it
// before stop()); stop() is synchronous so exit handlers can use it. `tag` is a string unique to this browser's
// command line (its profile dir): processes still carrying it after stop() are killed and counted.
export function launch(cmd, args, { cpus, maxRssMB, maxSeconds, tag, log }) {
  const unit = `bm-webperf-${process.pid}-${launched++}`;
  let child;
  let cgroup = null;
  let rss = null;
  // with a session bus, Chrome's browser process moves itself into its own app-org.chromium.Chromium-<pid>.scope
  const noBus = `DBUS_SESSION_BUS_ADDRESS=${NO_BUS}`;
  if (systemdScopeWorks()) {
    // --scope and env exec the command: child.pid is Chrome's own pid, inside the new cgroup
    child = spawn("systemd-run", ["--user", "--scope", "--quiet", "--collect", "--expand-environment=no",
      "--unit", unit, "--nice=10", "-p", `CPUQuota=${cpus * 100}%`, "-p", `MemoryMax=${maxRssMB}M`,
      "-p", "MemorySwapMax=0", "-p", "TasksMax=1024", "-p", `RuntimeMaxSec=${maxSeconds}`, "-p", "TimeoutStopSec=5",
      "--", "env", noBus, cmd, ...args], { stdio: "ignore" });
  } else {
    log("warning: systemd-run --user --scope unavailable: Chrome runs uncapped (RSS watch only, no CPU limit)");
    child = spawn(cmd, args, { stdio: "ignore", detached: true,
      env: { ...process.env, DBUS_SESSION_BUS_ADDRESS: NO_BUS } });
    rss = watchRss(child.pid, maxRssMB, (mb) => {
      log(`Chrome RSS ${mb} MB > --max-rss ${maxRssMB} MB: killing it`);
      killGroup(child.pid);
    });
  }
  const cgroupDir = () => {
    if (!scopeWorks || cgroup) return cgroup;
    try {
      const path = execFileSync("systemctl", ["--user", "show", "-p", "ControlGroup", "--value", `${unit}.scope`], quiet).trim();
      if (path) cgroup = `/sys/fs/cgroup${path}`;
    } catch {
      // scope not up yet or gone
    }
    return cgroup;
  };
  let stopped = false;
  const handle = {
    child,
    unit: scopeWorks ? unit : null,
    stats: () => scopeStats(cgroupDir(), rss),
    stop: () => {
      if (stopped) return handle.leftovers;
      stopped = true;
      rss?.stop();
      if (scopeWorks) {
        try {
          execFileSync("systemctl", ["--user", "stop", `${unit}.scope`], quiet);
        } catch {
          // already gone
        }
      } else {
        killGroup(child.pid);
      }
      const left = pgrep(tag);
      left.forEach((pid) => {
        try {
          process.kill(pid, "SIGKILL");
        } catch {
          // exited meanwhile
        }
      });
      live.delete(handle);
      handle.leftovers = left.length;
      return left.length;
    },
  };
  live.add(handle);
  return handle;
}

function scopeStats(dir, rss) {
  if (!dir) return { capped: false, memPeakMB: rss?.peak() ?? null };
  const cpu = (key) => Number(new RegExp(`${key} (\\d+)`).exec(read(`${dir}/cpu.stat`))?.[1] ?? NaN);
  try {
    return { capped: true, cpuMs: Math.round(cpu("usage_usec") / 1e3), throttledMs: Math.round(cpu("throttled_usec") / 1e3),
      memPeakMB: Math.round(Number(read(`${dir}/memory.peak`)) / 2 ** 20),
      oomKills: Number(/oom_kill (\d+)/.exec(read(`${dir}/memory.events`))?.[1] ?? 0) };
  } catch {
    return { capped: true }; // scope already gone
  }
}

// how one run's box looked: contended when cpu PSI some avg10 passed the threshold at its start or end
export const boxRecord = (before, after, stats, contendedPsi) => ({ before, after, ...stats,
  contended: Math.max(before.cpuPsiSome10, after.cpuPsiSome10) > contendedPsi });

export const stopAll = () => [...live].reduce((n, h) => n + h.stop(), 0);

// stops every live browser on exit, Ctrl-C, kill and crashes; RuntimeMaxSec covers a SIGKILLed node
export function installTeardown(log) {
  process.on("exit", stopAll);
  for (const [signal, code] of [["SIGINT", 130], ["SIGTERM", 143], ["SIGHUP", 129]]) {
    process.on(signal, () => {
      log(`${signal}: stopping Chrome (${live.size} scope(s))`);
      const left = stopAll();
      if (left) log(`killed ${left} Chrome process(es) left outside the scope`);
      process.exit(code);
    });
  }
  for (const event of ["uncaughtException", "unhandledRejection"]) {
    process.on(event, (e) => {
      log(`${event}: ${e?.stack ?? e}`);
      stopAll();
      process.exit(1);
    });
  }
}

// bm-webperf scopes of other run.mjs processes: [unit, owner pid]
function otherScopes() {
  let units = "";
  try {
    units = execFileSync("systemctl", ["--user", "list-units", "--all", "--plain", "--no-legend", "bm-webperf-*"], quiet);
  } catch {
    return [];
  }
  return [...units.matchAll(/(bm-webperf-(\d+)-\d+\.scope)/g)].map(([, name, pid]) => [name, Number(pid)])
    .filter(([, pid]) => pid !== process.pid);
}

// one capped Chrome per box (every session runs as the same user): waits, 5 s polls, while another run's scope lives
export async function waitForTurn(maxMs, log) {
  const until = Date.now() + maxMs;
  for (;;) {
    const busy = otherScopes().filter(([, pid]) => alive(pid));
    if (!busy.length) return true;
    if (Date.now() >= until) return false;
    log(`another webperf run is live (${busy.map(([n]) => n).join(", ")}): waiting`);
    await new Promise((r) => setTimeout(r, Math.min(5000, until - Date.now())));
  }
}

// scopes left by an earlier run.mjs that died (its pid is in the unit name)
export function reapStaleScopes(log) {
  for (const [name, pid] of otherScopes()) {
    if (alive(pid)) continue;
    log(`stopping stale ${name}`);
    try {
      execFileSync("systemctl", ["--user", "stop", name], quiet);
    } catch {
      // gone meanwhile
    }
  }
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === "EPERM";
  }
}

function killGroup(pid) {
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    // already gone
  }
}

function pgrep(pattern) {
  try {
    return execFileSync("pgrep", ["-f", pattern], quiet).trim().split("\n").filter(Boolean).map(Number);
  } catch {
    return []; // pgrep exits 1 when nothing matches
  }
}

// RSS in MB of `root` and every descendant
function treeRssMB(root) {
  let out;
  try {
    out = execFileSync("ps", ["-e", "-o", "pid=,ppid=,rss="], quiet);
  } catch {
    return 0;
  }
  const kids = new Map();
  const rss = new Map();
  for (const line of out.trim().split("\n")) {
    const [pid, ppid, kb] = line.trim().split(/\s+/).map(Number);
    rss.set(pid, kb);
    if (!kids.has(ppid)) kids.set(ppid, []);
    kids.get(ppid).push(pid);
  }
  let kb = 0;
  for (const stack = [root]; stack.length; ) {
    const p = stack.pop();
    kb += rss.get(p) ?? 0;
    stack.push(...(kids.get(p) ?? []));
  }
  return Math.round(kb / 1024);
}

// samples the tree every second and calls onLimit once past limitMB
function watchRss(root, limitMB, onLimit) {
  let peak = 0;
  let fired = false;
  const timer = setInterval(() => {
    const mb = treeRssMB(root);
    peak = Math.max(peak, mb);
    if (mb > limitMB && !fired) {
      fired = true;
      onLimit(mb);
    }
  }, 1000);
  return { peak: () => peak, stop: () => clearInterval(timer) };
}
