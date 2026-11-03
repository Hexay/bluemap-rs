// The summary rows of a run result (run.mjs) and their medians over runs.
export const ROWS = {
  "view ms": (r) => r.view.ms,
  "settle ms": (r) => r.settle?.ms,
  "main ms/frame p50": (r) => r.renderLoopMs.p50,
  "main ms/frame p95": (r) => r.renderLoopMs.p95,
  "main busy ms/s": (r) => r.cdp.taskMsPerS,
  "terrainHeight ms/frame p95": (r) => r.terrainHeightMs.p95,
  "draw calls p50": (r) => r.drawCalls.p50,
  "triangles k p50": (r) => r.trianglesK.p50,
  "long tasks": (r) => r.longTasks.n,
  "longest task ms": (r) => r.longTasks.max,
  "hires tile ms p50": (r) => r.tileMs.hires.p50,
  "hires tile ms p95": (r) => r.tileMs.hires.p95,
  "lowres tile ms p50": (r) => r.tileMs.lowres.p50,
  "lowres tile ms p95": (r) => r.tileMs.lowres.p95,
  "tiles in flight/frame mean": (r) => r.inFlight.mean,
  "tile starts/rendered frame": (r) => r.tileStarts.perRenderedFrame,
  "tiles fetched": (r) => r.net.hires.n + r.net.lowres.n,
  "kB fetched": (r) => r.net.hires.kB + r.net.lowres.kB,
  "heap MB peak": (r) => r.memory.peakHeapMB,
  "geometries end": (r) => r.memory.end?.geometries,
  "textures end": (r) => r.memory.end?.textures,
  "hires geom MB peak": (r) => r.memory.peakHiresGeomMB,
  "chrome RSS MB peak": (r) => r.chromeRssPeakMB,
  "rAF fps (software GL)": (r) => r.fps,
  "rendered fps (software GL)": (r) => r.renderedFps,
  "frame p95 ms (software GL)": (r) => r.frameInterval.p95,
  "frames >50ms (software GL)": (r) => r.frameInterval.over50,
};

export const median = (xs) => {
  const v = xs.filter((x) => typeof x === "number" && !Number.isNaN(x)).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)] : null;
};

// { row: { column: median } } over results grouped by column(r)
export function medianTable(results, column) {
  const groups = new Map();
  for (const r of results) {
    const c = column(r);
    if (!groups.has(c)) groups.set(c, []);
    groups.get(c).push(r);
  }
  const table = {};
  for (const [k, f] of Object.entries(ROWS)) {
    table[k] = {};
    for (const [c, rs] of groups) {
      table[k][c] = median(rs.map((r) => {
        try {
          return f(r);
        } catch {
          return null;
        }
      }));
    }
  }
  return table;
}
