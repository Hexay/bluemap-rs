// Self time of a DevTools Profiler.Profile, bucketed by script and ranked by function.
const BUCKETS = [
  ["webapp+three", (u) => /\/assets\/index-[^/]*\.js/.test(u)],
  ["unpack.js", (u) => /bluemap-rs-unpack/.test(u)],
  ["other script", (u) => u !== ""],
];

export function summarizeProfile(profile, top = 15) {
  const self = new Map(profile.nodes.map((n) => [n.id, 0]));
  const { samples, timeDeltas } = profile;
  for (let i = 0; i < samples.length; i++) {
    // the delta *after* a sample is the time it stood for
    const dt = (timeDeltas[i + 1] ?? 0) / 1000;
    self.set(samples[i], self.get(samples[i]) + dt);
  }
  const buckets = {};
  const fns = new Map();
  let total = 0;
  for (const n of profile.nodes) {
    const ms = self.get(n.id);
    if (!ms) continue;
    const { functionName, url, lineNumber, columnNumber } = n.callFrame;
    const special = !url && /^\(.*\)$/.test(functionName) ? functionName : null;
    const bucket = special ?? BUCKETS.find(([, test]) => test(url))?.[0] ?? "native/other";
    if (bucket !== "(idle)") total += ms;
    buckets[bucket] = (buckets[bucket] ?? 0) + ms;
    const key = special ?? `${functionName || "(anonymous)"} ${url.split("/").pop()}:${lineNumber + 1}:${columnNumber + 1}`;
    fns.set(key, (fns.get(key) ?? 0) + ms);
  }
  const r = (x) => Math.round(x);
  return {
    busyMs: r(total),
    buckets: Object.fromEntries(Object.entries(buckets).sort((a, b) => b[1] - a[1]).map(([k, v]) => [k, r(v)])),
    top: [...fns.entries()].filter(([k]) => k !== "(idle)").sort((a, b) => b[1] - a[1]).slice(0, top)
      .map(([fn, ms]) => ({ fn, ms: r(ms), pct: Math.round((1000 * ms) / total) / 10 })),
  };
}
