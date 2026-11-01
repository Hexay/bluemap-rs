// The Windows host side of smoothness.mjs: is the machine quiet, and what do our Chrome's processes hold.
import { spawnSync } from "node:child_process";
import { cpus, freemem } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";

const SNAPSHOT = `Get-CimInstance Win32_Process | ForEach-Object {
  "$($_.ProcessId)\`t$($_.KernelModeTime + $_.UserModeTime)\`t$($_.WorkingSetSize)\`t$($_.PrivatePageCount)\`t$($_.CommandLine)" }`;

// pid -> { cpuS, ws, priv, cmd }
export function processes() {
  const out = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", SNAPSHOT], { encoding: "utf8", maxBuffer: 1 << 26 });
  const all = new Map();
  for (const line of out.stdout.split(/\r?\n/)) {
    const [pid, cpu, ws, priv, cmd] = line.split("\t");
    // pid 0 is the idle process; a command line may hold newlines, so some lines are its tail
    if (/^[1-9]\d*$/.test(pid) && /^\d+$/.test(cpu)) all.set(Number(pid), { cpuS: Number(cpu) / 1e7, ws: Number(ws), priv: Number(priv), cmd: cmd ?? "" });
  }
  return all;
}

// MB per Chrome process type of the profile; browser = no --type=
export function chromeMemory(snapshot, profile) {
  const mb = {};
  for (const p of snapshot.values()) {
    if (!p.cmd.includes(profile)) continue;
    const type = /--type=([\w-]+)/.exec(p.cmd)?.[1] ?? "browser";
    const kind = /--utility-sub-type=network/.test(p.cmd) ? "network" : type;
    mb[kind] = Math.round((mb[kind] ?? 0) + p.ws / 1048576);
  }
  return mb;
}

// Share of the machine's capacity, 0..1, used between two snapshots by everyone except the bench itself.
export function outsideLoad(before, after, seconds, isBench) {
  let cpu = 0;
  const top = [];
  for (const [pid, p] of after) {
    const b = before.get(pid);
    if (!b || isBench(pid, p)) continue;
    const used = p.cpuS - b.cpuS;
    cpu += used;
    if (used > 0.05 * seconds) top.push(`${/[^\\/"]+\.exe/i.exec(p.cmd)?.[0] ?? pid}:${used.toFixed(1)}s`);
  }
  return { share: cpu / (seconds * cpus().length), top };
}

export async function totalBusy(ms) {
  const read = () => cpus().reduce((n, c) => ({ idle: n.idle + c.times.idle,
    total: n.total + c.times.user + c.times.sys + c.times.idle + c.times.irq + c.times.nice }), { idle: 0, total: 0 });
  const a = read();
  await sleep(ms);
  const b = read();
  return 1 - (b.idle - a.idle) / (b.total - a.total);
}

// Waits for a quiet machine, null if it stays busy for maxWaitS.
export async function waitQuiet(maxBusy, minFreeMb, maxWaitS) {
  const end = Date.now() + maxWaitS * 1000;
  for (;;) {
    const busy = await totalBusy(3000);
    const freeMb = Math.round(freemem() / 1048576);
    if (busy <= maxBusy && freeMb >= minFreeMb) return { busy, freeMb };
    if (Date.now() > end) return null;
    console.log(`waiting for a quiet machine: busy ${(busy * 100).toFixed(0)}%, free ${freeMb} MB`);
    await sleep(15000);
  }
}

export const onBattery = () => {
  const out = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
    "(Get-CimInstance Win32_Battery).BatteryStatus"], { encoding: "utf8" }).stdout.trim();
  return out === "1";
};
