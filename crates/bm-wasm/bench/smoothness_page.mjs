// What smoothness.mjs evaluates in the page. Motion follows the wall clock, so every run asks for the same camera at the
// same time and a stalled frame jumps, as a user's drag would feel it.
export const RECORDER = `(() => {
  const s = window.__smooth = { frames: new Float64Array(1 << 19), n: 0, tasks: [], hiddenAt: [] };
  const tick = (t) => {
    if (s.n < s.frames.length) s.frames[s.n++] = t;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  new PerformanceObserver((list) => list.getEntries().forEach((e) => s.tasks.push([e.startTime, e.duration])))
    .observe({ type: "longtask", buffered: true });
  addEventListener("visibilitychange", () => document.hidden && s.hiddenAt.push(performance.now()));
  if (document.hidden) s.hiddenAt.push(0);
})()`;

export const NOW = "performance.now()";

// frame intervals and long tasks in [t0, t1], plus what can void the run
export const slice = (t0, t1) => `(() => {
  const s = window.__smooth;
  const intervals = [];
  for (let i = 1; i < s.n; i++) if (s.frames[i - 1] >= ${t0} && s.frames[i] <= ${t1}) intervals.push(s.frames[i] - s.frames[i - 1]);
  const tasks = s.tasks.filter(([start]) => start >= ${t0} && start <= ${t1}).map(([, d]) => d);
  return JSON.stringify({ intervals, tasks, hiddenAt: s.hiddenAt, hidden: document.hidden, focus: document.hasFocus() });
})()`;

// WEBGL_debug_renderer_info of the webapp's own context, once it has one
export const RENDERER = `(() => {
  const r = window.bluemap?.mapViewer?.renderer;
  const gl = r ? r.getContext() : document.createElement("canvas").getContext("webgl2");
  const info = gl?.getExtension("WEBGL_debug_renderer_info");
  return JSON.stringify({ renderer: info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "no WEBGL_debug_renderer_info",
    dpr: devicePixelRatio, canvas: r ? [gl.drawingBufferWidth, gl.drawingBufferHeight] : null,
    cores: navigator.hardwareConcurrency });
})()`;

export const teleport = (x, z) => `(() => {
  const c = window.bluemap.mapViewer.controlsManager;
  c.position.x = ${x};
  c.position.z = ${z};
})()`;

// Moves along a polyline at constant speed for `ms`, then waits for the hires manager to go quiet. A tile counts as
// entered when it first falls inside the manager's square, not when it starts loading: the manager queues 8 in flight
// and its spiral never reaches some corners, so `never` is normal. Tiles already in the square are ignored.
export const pan = (points, ms, settleMs) => `new Promise((resolve) => {
  const points = ${JSON.stringify(points)};
  const lengths = points.slice(1).map((p, i) => Math.hypot(p[0] - points[i][0], p[1] - points[i][1]));
  const total = lengths.reduce((a, b) => a + b, 0);
  const at = (u) => {
    let d = u * total, i = 0;
    while (i < lengths.length - 1 && d > lengths[i]) d -= lengths[i++];
    const f = d / lengths[i];
    return [points[i][0] + (points[i + 1][0] - points[i][0]) * f, points[i][1] + (points[i + 1][1] - points[i][1]) * f];
  };
  const c = window.bluemap.mapViewer.controlsManager;
  const m = window.bluemap.mapViewer.map.hiresTileManager;
  const seen = new Map();
  const scan = (now) => {
    const live = new Map();
    m.tiles.forEach((t) => live.set(t.x + "," + t.z, t));
    const dx = m.viewDistanceX, dz = m.viewDistanceZ;
    for (let x = m.centerTile.x - dx; x <= m.centerTile.x + dx; x++) for (let z = m.centerTile.y - dz; z <= m.centerTile.y + dz; z++) {
      const k = x + "," + z;
      if (!seen.has(k)) seen.set(k, { entered: now, loaded: live.get(k)?.model ? now : null, had: !!live.get(k)?.model });
    }
    seen.forEach((r, k) => { if (r.loaded === null && live.get(k)?.model) r.loaded = now; });
  };
  scan(performance.now());
  seen.forEach((r) => (r.had = true));
  const t0 = performance.now();
  let t1 = null, quiet = 0;
  const step = (now) => {
    now = performance.now();
    const u = (now - t0) / ${ms};
    if (u < 1) {
      const [x, z] = at(u);
      c.position.x = x;
      c.position.z = z;
    } else if (t1 === null) {
      const [x, z] = at(1);
      c.position.x = x;
      c.position.z = z;
      t1 = now;
    }
    scan(now);
    quiet = t1 !== null && m.currentlyLoading === 0 ? quiet + 1 : 0;
    if (quiet >= 3 || (t1 !== null && now - t1 > ${settleMs})) {
      const fresh = [...seen.values()].filter((r) => !r.had);
      const done = fresh.filter((r) => r.loaded !== null);
      return resolve(JSON.stringify({ t0, t1, settleMs: now - t1, latencies: done.map((r) => r.loaded - r.entered),
        lastLoadedAfterEnd: done.length ? Math.max(...done.map((r) => r.loaded)) - t1 : null,
        never: fresh.length - done.length, position: [c.position.x, c.position.z] }));
    }
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
})`;

// from the view's distance out to `far` and back, geometric, on a sine so it eases at both ends
export const zoom = (far, ms) => `new Promise((resolve) => {
  const c = window.bluemap.mapViewer.controlsManager;
  const near = c.distance;
  let peak = near;
  const t0 = performance.now();
  const step = () => {
    const now = performance.now();
    const u = Math.min((now - t0) / ${ms}, 1);
    c.distance = near * Math.pow(${far} / near, Math.sin(Math.PI * u));
    peak = Math.max(peak, c.distance);
    if (u < 1) return requestAnimationFrame(step);
    c.distance = near;
    resolve(JSON.stringify({ t0, t1: now, peak, end: c.distance }));
  };
  requestAnimationFrame(step);
})`;
