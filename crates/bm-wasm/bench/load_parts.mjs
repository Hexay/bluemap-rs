// Analysis behind load_ab.mjs: what the browser's network stack saw per category, and a wall-clock partition of a
// load (main thread busy / waiting on the network / workers only / nothing) from network events plus a Chrome trace.
const lodOf = (path) => path.match(/\/tiles\/(\d+)\//)?.[1];

export function category(url) {
  const path = new URL(url).pathname;
  const lod = lodOf(path);
  if (lod === "0") return "hires";
  if (lod) return "lowres";
  if (path.includes("/live/")) return "live";
  if (path.includes("tile-versions")) return "tile-versions";
  if (path.endsWith("/textures.json") || path.endsWith("/textures.json.gz")) return "textures";
  if (path.endsWith("/settings.json")) return "settings";
  if (/(^\/|\.html)$/.test(path)) return "html";
  return "static";
}

const median = (xs) => (xs.length ? xs.toSorted((a, b) => a - b)[xs.length >> 1] : null);
const round = (x) => (x === null ? null : Math.round(x * 10) / 10);

// total covered length of [start, end] intervals, in the unit given
function union(intervals) {
  let total = 0, until = -Infinity;
  for (const [a, b] of intervals.toSorted((p, q) => p[0] - q[0])) {
    if (b <= until) continue;
    total += b - Math.max(a, until);
    until = b;
  }
  return total;
}

// requests: { url, start, sendAt, headersAt, end, status, bytes, cached, conditional, failed } with times in seconds
export function requestStats(requests) {
  const out = {};
  for (const [name, rs] of Map.groupBy(requests, (r) => category(r.url))) {
    const net = rs.filter((r) => !r.cached);
    const timed = net.filter((r) => r.sendAt && r.headersAt);
    const statuses = {};
    for (const r of rs) {
      const s = r.cached ? "cache" : r.failed ? "aborted" : r.status ?? "?";
      statuses[s] = (statuses[s] ?? 0) + 1;
    }
    out[name] = {
      n: rs.length, statuses, conditional: rs.filter((r) => r.conditional).length,
      kb: round(net.reduce((n, r) => n + (r.bytes ?? 0), 0) / 1024),
      queueMs: round(median(timed.map((r) => (r.sendAt - r.start) * 1e3))),
      queueSumMs: Math.round(timed.reduce((n, r) => n + (r.sendAt - r.start) * 1e3, 0)),
      rttMs: round(median(timed.map((r) => (r.headersAt - r.sendAt) * 1e3))),
      rttSumMs: Math.round(timed.reduce((n, r) => n + (r.headersAt - r.sendAt) * 1e3, 0)),
      wallMs: Math.round(union(rs.filter((r) => r.end).map((r) => [r.start, r.end])) * 1e3),
    };
  }
  return out;
}

// Milestones of a load relative to t0 (seconds), hires tiles in particular.
export function milestones(requests, t0) {
  const hires = requests.filter((r) => category(r.url) === "hires");
  const tiles = requests.filter((r) => /\/tiles\//.test(r.url));
  const rel = (x) => Math.round((x - t0) * 1e3);
  const at = (rs, f, pick) => (rs.length ? rel(pick(...rs.map(f))) : null);
  return {
    firstTile: at(tiles, (r) => r.start, Math.min), firstHires: at(hires, (r) => r.start, Math.min),
    lastHiresStart: at(hires, (r) => r.start, Math.max), lastHiresEnd: at(hires.filter((r) => r.end), (r) => r.end, Math.max),
    lastEnd: at(requests.filter((r) => r.end), (r) => r.end, Math.max),
  };
}

const threadsNamed = (events, re) => {
  const names = new Map();
  for (const e of events) if (e.ph === "M" && e.name === "thread_name") names.set(`${e.pid}:${e.tid}`, e.args.name);
  const groups = new Map();
  for (const e of events) {
    if (e.ph !== "X" || e.dur === undefined || !re.test(names.get(`${e.pid}:${e.tid}`) ?? "")) continue;
    const key = `${e.pid}:${e.tid}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push([e.ts / 1e6, (e.ts + e.dur) / 1e6]);
  }
  return [...groups.values()];
};

function clip(intervals, t0, t1) {
  return intervals.filter(([a, b]) => b > t0 && a < t1).map(([a, b]) => [Math.max(a, t0), Math.min(b, t1)]);
}

// Each millisecond of [t0, t1] goes to the first that holds: main thread busy, a request in flight, a worker busy, else idle.
export function partition(events, requests, t0, t1) {
  const mains = threadsNamed(events, /^CrRendererMain$/);
  const main = clip(mains.toSorted((a, b) => union(b) - union(a))[0] ?? [], t0, t1);
  const workers = clip(threadsNamed(events, /^DedicatedWorker/).flat(), t0, t1);
  const gpu = clip(threadsNamed(events, /^CrGpuMain$/).flat(), t0, t1);
  const inFlight = clip(requests.filter((r) => r.end).map((r) => [r.start, r.end]), t0, t1);
  const hiresFlight = clip(requests.filter((r) => r.end && category(r.url) === "hires").map((r) => [r.start, r.end]), t0, t1);
  const n = Math.ceil((t1 - t0) * 1e3);
  const mark = (intervals) => {
    const bits = new Uint8Array(n + 1);
    for (const [a, b] of intervals) bits.fill(1, Math.floor((a - t0) * 1e3), Math.ceil((b - t0) * 1e3));
    return bits;
  };
  const [m, f, w] = [mark(main), mark(inFlight), mark(workers)];
  const parts = { mainBusy: 0, networkWait: 0, workersOnly: 0, idle: 0, mainAndNetwork: 0 };
  for (let i = 0; i < n; i++) {
    if (m[i]) parts.mainBusy++;
    else if (f[i]) parts.networkWait++;
    else if (w[i]) parts.workersOnly++;
    else parts.idle++;
    if (m[i] && f[i]) parts.mainAndNetwork++;
  }
  return { totalMs: n, ...parts, workerBusyMs: Math.round(union(workers) * 1e3), gpuBusyMs: Math.round(union(gpu) * 1e3),
    hiresWallMs: Math.round(union(hiresFlight) * 1e3) };
}
