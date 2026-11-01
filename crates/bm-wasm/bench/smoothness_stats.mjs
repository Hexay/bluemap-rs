// Per-phase numbers of one run and the medians-with-ranges tables of smoothness.mjs --report.
import { readFileSync } from "node:fs";

const percentile = (sorted, p) => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)] : null);
const round = (x, d = 1) => (x === null || x === undefined ? null : Number(x.toFixed(d)));

export const THRESHOLDS = [12.5, 16.7, 25, 33, 50];

export function frameStats(intervals, tasks) {
  const sorted = intervals.toSorted((a, b) => a - b);
  const over = Object.fromEntries(THRESHOLDS.map((t) => [t, round((intervals.filter((i) => i > t).length * 100) / Math.max(1, intervals.length), 2)]));
  return { frames: intervals.length, p50: round(percentile(sorted, 50), 2), p95: round(percentile(sorted, 95), 2),
    p99: round(percentile(sorted, 99), 2), max: round(sorted.at(-1) ?? null, 1), over,
    longestTask: round(Math.max(0, ...tasks), 0), longTasks: tasks.length };
}

export const latencyStats = (ms) => {
  const sorted = ms.toSorted((a, b) => a - b);
  return { tiles: ms.length, p50: round(percentile(sorted, 50), 0), p95: round(percentile(sorted, 95), 0), max: round(sorted.at(-1) ?? null, 0) };
};

const median = (xs) => {
  const s = xs.filter((x) => x !== null && x !== undefined).toSorted((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : null;
};
// "median (min–max)"
function cell(xs, digits = 1) {
  const s = xs.filter((x) => x !== null && x !== undefined);
  if (!s.length) return "-";
  const f = (x) => (digits === 0 ? Math.round(x).toLocaleString("en-US") : x.toFixed(digits));
  return `${f(median(s))} (${f(Math.min(...s))}–${f(Math.max(...s))})`;
}

const PHASES = { load: "(a) first load", idle: "(b) idle 5 s", pan: "(c) pan, new tiles", panAgain: "(d) same pan, seen tiles",
  zoom: "(e) zoom out and in", reload: "(f) reload" };
const ROW_ORDER = ["j529", "rc", "ro", "rn"];

function table(title, header, rows) {
  console.log(`\n${title}\n\n| ${header.join(" | ")} |\n|${header.map((_, i) => (i ? "---:" : "---")).join("|")}|`);
  for (const r of rows) console.log(`| ${r.join(" | ")} |`);
}

export function report(file) {
  const all = readFileSync(file, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  const valid = all.filter((r) => r.valid);
  console.log(`${valid.length} valid runs of ${all.length}; renderers: ${[...new Set(valid.map((r) => r.gpu.renderer))].join(", ")}; ` +
    `dpr ${[...new Set(valid.map((r) => r.gpu.dpr))]}, canvas ${[...new Set(valid.map((r) => r.gpu.canvas?.join("x")))]}`);
  const voided = all.filter((r) => !r.valid);
  if (voided.length) console.log(`voided: ${voided.map((r) => `${r.row}/${r.net}/${r.run}: ${r.why}`).join("; ")}`);
  for (const net of [...new Set(valid.map((r) => r.net))]) {
    const of = (row) => valid.filter((r) => r.net === net && r.row === row);
    const rows = ROW_ORDER.filter((row) => of(row).length);
    const header = (r) => `${r} (${of(r).length})`;
    table(`### ${net} ms,Mbit/s: tiles in`, ["row (runs)", "first load ms", "all requests done ms", "reload ms", "tunnel RTT ms", "outside load %", "tunnel ssh CPU %"],
      rows.map((r) => [header(r), cell(of(r).map((x) => x.phases.load.ms), 0), cell(of(r).map((x) => x.allInMs), 0), cell(of(r).map((x) => x.phases.reload.ms), 0),
        cell(of(r).map((x) => x.tunnelRttMs)), cell(of(r).map((x) => x.outside.share * 100)), cell(of(r).map((x) => x.tunnelCpuPct))]));
    table(`### ${net}: time from a tile entering the area until loaded`,
      ["row", "phase", "tiles", "p50 ms", "p95 ms", "max ms", "never", "last tile after pan end ms"],
      rows.flatMap((r) => ["pan", "panAgain"].map((p) => {
        const ps = of(r).map((x) => x.phases[p]);
        return [r, p === "pan" ? "(c)" : "(d)", cell(ps.map((x) => x.tileLatency.tiles), 0), cell(ps.map((x) => x.tileLatency.p50), 0),
          cell(ps.map((x) => x.tileLatency.p95), 0), cell(ps.map((x) => x.tileLatency.max), 0), cell(ps.map((x) => x.never), 0),
          cell(ps.map((x) => x.lastLoadedAfterEnd), 0)];
      })));
    for (const [phase, title] of Object.entries(PHASES)) {
      const ps = (r) => of(r).map((x) => x.phases[phase]);
      table(`### ${net}: ${title}`, ["row", "frames", "p50", "p95", "p99", "max", ">16.7 %", ">33 %", ">50 %", "longest task", "main busy ms",
        "JS heap MB", "worker heap MB", "renderer MB", "GPU proc MB"], rows.map((r) => [r,
        cell(ps(r).map((x) => x.frames), 0), cell(ps(r).map((x) => x.p50)), cell(ps(r).map((x) => x.p95)), cell(ps(r).map((x) => x.p99)),
        cell(ps(r).map((x) => x.max), 0), cell(ps(r).map((x) => x.over["16.7"]), 2), cell(ps(r).map((x) => x.over["33"]), 2),
        cell(ps(r).map((x) => x.over["50"]), 2), cell(ps(r).map((x) => x.longestTask), 0), cell(ps(r).map((x) => x.mainMs), 0),
        cell(ps(r).map((x) => x.memory.jsHeapMb), 0), cell(ps(r).map((x) => x.memory.workerHeapMb), 0),
        cell(ps(r).map((x) => x.memory.chrome.renderer), 0), cell(ps(r).map((x) => x.memory.chrome["gpu-process"]), 0)]));
    }
  }
}
