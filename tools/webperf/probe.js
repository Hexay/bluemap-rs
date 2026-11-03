// In-page performance probe for the BlueMap webapp. Works injected by run.mjs or pasted into a DevTools console:
//   bmPerf.start(); /* move around */ bmPerf.stop(); console.table(bmPerf.report().table)
// Times the webapp's own render loop (main-thread ms per frame, GPU-independent), rAF intervals (frame pacing, real
// FPS only on a real GPU), long animation frames with script attribution, draw calls/triangles summed over the
// webapp's several renderer.render calls per frame, GPU-side geometry/texture counts, JS heap, tile counts and tile
// latency (request to model in the scene).
(() => {
  if (window.bmPerf) return;
  const now = () => performance.now();
  const viewer = () => window.bluemap?.mapViewer;
  let s = null; // the running session
  let hooked = null; // the viewer whose functions are wrapped
  const tileLoaders = new WeakSet();

  const pct = (xs, p) => {
    if (!xs.length) return 0;
    const v = [...xs].sort((a, b) => a - b);
    return v[Math.min(v.length - 1, Math.floor((p / 100) * v.length))];
  };
  const round = (x, d = 2) => Math.round(x * 10 ** d) / 10 ** d;

  function hookViewer(v) {
    if (hooked === v) return;
    hooked = v;
    const renderer = v.renderer;
    renderer.info.autoReset = false; // the webapp renders 4+ scenes per frame; sum them, reset per frame below
    const loop = v.renderLoop;
    v.renderLoop = (t) => {
      if (!s) return loop(t);
      const calls0 = renderer.info.render.calls, tris0 = renderer.info.render.triangles;
      const f0 = renderer.info.render.frame;
      const t0 = now();
      s.heightAcc = 0;
      loop(t);
      const dt = now() - t0;
      s.loopMs.push(dt);
      s.heightMs.push(s.heightAcc);
      if (renderer.info.render.frame !== f0) {
        s.renderMs.push(dt);
        s.calls.push(renderer.info.render.calls - calls0);
        s.tris.push(renderer.info.render.triangles - tris0);
      }
      if (renderer.info.render.calls > 1e9) renderer.info.reset();
    };
  }

  // Wraps every method of the webapp's WebGL context (instance properties, so other contexts are untouched): ms and
  // calls per GL method while a session with {gl: true} runs. Calls that wait for the GPU process show up here.
  let glHooked = null;
  function hookGl(gl) {
    if (glHooked === gl) return;
    glHooked = gl;
    for (let p = Object.getPrototypeOf(gl); p && p !== Object.prototype; p = Object.getPrototypeOf(p)) {
      for (const name of Object.getOwnPropertyNames(p)) {
        const d = Object.getOwnPropertyDescriptor(p, name);
        if (typeof d.value !== "function" || name === "constructor" || Object.hasOwn(gl, name)) continue;
        const fn = d.value;
        gl[name] = function (...a) {
          if (!s?.gl) return fn.apply(this, a);
          const t0 = now();
          try {
            return fn.apply(this, a);
          } finally {
            const c = s.gl.get(name) ?? { ms: 0, n: 0 };
            c.ms += now() - t0;
            c.n++;
            s.gl.set(name, c);
          }
        };
      }
    }
  }

  function hookTileLoader(manager, kind) {
    const loader = manager?.tileLoader;
    if (!loader || tileLoaders.has(loader)) return;
    tileLoaders.add(loader);
    const load = loader.load.bind(loader);
    loader.load = (...args) => {
      const t0 = now();
      const p = load(...args);
      p.then(() => s?.tileMs[kind].push(now() - t0), () => s && s.tileFail[kind]++);
      return p;
    };
  }

  function hookMap() {
    const v = viewer();
    if (!v?.renderer) return false;
    hookViewer(v);
    if (s?.gl) hookGl(v.renderer.getContext());
    const map = v.map;
    if (map && !map.__bmPerfHooked) {
      // MapHeightControls calls this twice per frame: a raycast into the hires tile, else a lowres getImageData
      map.__bmPerfHooked = true;
      const height = map.terrainHeightAt.bind(map);
      map.terrainHeightAt = (x, z) => {
        if (!s) return height(x, z);
        const t0 = now();
        const h = height(x, z);
        s.heightAcc += now() - t0;
        return h;
      };
    }
    if (map?.hiresTileManager) {
      hookTileLoader(map.hiresTileManager, "hires");
      (map.lowresTileManager ?? []).forEach((m) => hookTileLoader(m, "lowres"));
    }
    return true;
  }

  // missing: neither loaded nor loading (no such tile on the server, beyond the map's edge)
  const countTiles = (m) => {
    let loaded = 0, loading = 0;
    m?.tiles?.forEach((t) => (t.model ? loaded++ : t.loading && loading++));
    const wanted = m?.tiles?.size ?? 0;
    return { wanted, loaded, loading, missing: wanted - loaded - loading };
  };

  function geometryBytes(scene) {
    let bytes = 0;
    const seen = new Set();
    scene?.traverse((o) => {
      const g = o.geometry;
      if (!g || seen.has(g)) return;
      seen.add(g);
      for (const a of Object.values(g.attributes)) bytes += a.array.byteLength;
      if (g.index) bytes += g.index.array.byteLength;
    });
    return bytes;
  }

  function sample() {
    hookMap();
    const v = viewer();
    const map = v?.map;
    const mem = v?.renderer?.info.memory;
    const lowres = (map?.lowresTileManager ?? []).map(countTiles);
    s.samples.push({
      t: round((now() - s.t0) / 1000, 1),
      heapMB: performance.memory ? round(performance.memory.usedJSHeapSize / 2 ** 20, 1) : null,
      geometries: mem?.geometries ?? null,
      textures: mem?.textures ?? null,
      programs: v?.renderer?.info.programs?.length ?? null,
      hiresGeomMB: round(geometryBytes(map?.hiresTileManager?.scene) / 2 ** 20, 1),
      hires: countTiles(map?.hiresTileManager),
      lowres: lowres.reduce((a, b) => ({ wanted: a.wanted + b.wanted, loaded: a.loaded + b.loaded,
        loading: a.loading + b.loading }), { wanted: 0, loaded: 0, loading: 0 }),
    });
  }

  function observe(type, fn) {
    try {
      const o = new PerformanceObserver((list) => s && list.getEntries().forEach(fn));
      o.observe({ type, buffered: false });
      return o;
    } catch {
      return null; // unsupported entry type (Firefox: long-animation-frame)
    }
  }

  function start(opts = {}) {
    s = { t0: now(), gl: opts.gl ? new Map() : null, raf: [], loopMs: [], heightMs: [], heightAcc: 0, renderMs: [], calls: [], tris: [], longTasks: [], loaf: [],
      scripts: new Map(), tileMs: { hires: [], lowres: [] }, tileFail: { hires: 0, lowres: 0 },
      net: { hires: [], lowres: [], other: 0 }, samples: [], observers: [], timer: null, rafId: 0 };
    hookMap();
    let last = now();
    const tick = (t) => {
      if (!s) return;
      s.raf.push(t - last);
      last = t;
      s.rafId = requestAnimationFrame(tick);
    };
    s.rafId = requestAnimationFrame(tick);
    s.observers.push(observe("longtask", (e) => s.longTasks.push(e.duration)));
    s.observers.push(observe("long-animation-frame", (e) => {
      s.loaf.push({ duration: e.duration, blocking: e.blockingDuration });
      for (const sc of e.scripts ?? []) {
        const key = `${sc.invoker || sc.invokerType} @ ${(sc.sourceURL || "").split("/").pop()}:${sc.sourceFunctionName || ""}`;
        const cur = s.scripts.get(key) ?? { ms: 0, n: 0 };
        cur.ms += sc.duration;
        cur.n++;
        s.scripts.set(key, cur);
      }
    }));
    s.observers.push(observe("resource", (e) => {
      const m = /\/tiles\/(\d+)\//.exec(e.name);
      if (!m) return void s.net.other++;
      s.net[m[1] === "0" ? "hires" : "lowres"].push({ ms: e.duration, ttfb: e.responseStart - e.requestStart,
        bytes: e.transferSize, body: e.encodedBodySize });
    }));
    sample();
    s.timer = setInterval(sample, 1000);
    return "bmPerf started";
  }

  function stop() {
    if (!s) return null;
    sample();
    cancelAnimationFrame(s.rafId);
    clearInterval(s.timer);
    s.observers.forEach((o) => o?.disconnect());
    s.seconds = (now() - s.t0) / 1000;
    const done = s;
    s = null;
    window.bmPerf.last = done;
    return "bmPerf stopped";
  }

  const dist = (xs) => ({ n: xs.length, p50: round(pct(xs, 50)), p95: round(pct(xs, 95)), p99: round(pct(xs, 99)),
    max: round(Math.max(0, ...xs)), mean: round(xs.reduce((a, b) => a + b, 0) / (xs.length || 1)) });
  const netSummary = (rs) => ({ n: rs.length, kB: round(rs.reduce((a, r) => a + r.bytes, 0) / 1024, 0),
    p50ms: round(pct(rs.map((r) => r.ms), 50)), p95ms: round(pct(rs.map((r) => r.ms), 95)),
    ttfbP50: round(pct(rs.map((r) => r.ttfb), 50)) });

  function report() {
    const r = s ?? window.bmPerf.last;
    if (!r) return null;
    const seconds = r.seconds ?? (now() - r.t0) / 1000;
    const first = r.samples[0], last = r.samples[r.samples.length - 1];
    const out = {
      seconds: round(seconds, 1),
      fps: round(r.raf.length / seconds, 1),
      renderedFps: round(r.renderMs.length / seconds, 1),
      frameInterval: { ...dist(r.raf), over17: r.raf.filter((x) => x > 17.5).length,
        over33: r.raf.filter((x) => x > 34).length, over50: r.raf.filter((x) => x > 51).length },
      renderLoopMs: dist(r.renderMs),
      mainPerFrameMs: round(r.renderMs.reduce((a, b) => a + b, 0) / (r.renderMs.length || 1)),
      terrainHeightMs: dist(r.heightMs),
      drawCalls: dist(r.calls),
      trianglesK: dist(r.tris.map((x) => x / 1000)),
      longTasks: { n: r.longTasks.length, totalMs: round(r.longTasks.reduce((a, b) => a + b, 0), 0),
        max: round(Math.max(0, ...r.longTasks), 0) },
      longFrames: { n: r.loaf.length, blockingMs: round(r.loaf.reduce((a, b) => a + b.blocking, 0), 0) },
      topScripts: [...r.scripts.entries()].sort((a, b) => b[1].ms - a[1].ms).slice(0, 8)
        .map(([k, v]) => ({ script: k, ms: round(v.ms, 0), n: v.n })),
      gl: r.gl && [...r.gl.entries()].sort((a, b) => b[1].ms - a[1].ms).slice(0, 10)
        .map(([fn, v]) => ({ fn, ms: round(v.ms, 0), n: v.n, perCall: round(v.ms / v.n, 3) })),
      tileMs: { hires: dist(r.tileMs.hires), lowres: dist(r.tileMs.lowres), failed: r.tileFail },
      net: { hires: netSummary(r.net.hires), lowres: netSummary(r.net.lowres), other: r.net.other },
      memory: { start: first, end: last,
        peakHeapMB: Math.max(0, ...r.samples.map((x) => x.heapMB ?? 0)),
        peakGeometries: Math.max(0, ...r.samples.map((x) => x.geometries ?? 0)),
        peakTextures: Math.max(0, ...r.samples.map((x) => x.textures ?? 0)),
        peakHiresGeomMB: Math.max(0, ...r.samples.map((x) => x.hiresGeomMB)) },
      samples: r.samples,
    };
    out.table = {
      "fps (rAF)": out.fps, "rendered fps": out.renderedFps,
      "frame p50/p95/p99 ms": `${out.frameInterval.p50}/${out.frameInterval.p95}/${out.frameInterval.p99}`,
      "frames >33/>50 ms": `${out.frameInterval.over33}/${out.frameInterval.over50}`,
      "main ms/frame p50/p95": `${out.renderLoopMs.p50}/${out.renderLoopMs.p95}`,
      "draw calls p50/max": `${out.drawCalls.p50}/${out.drawCalls.max}`,
      "triangles k p50": out.trianglesK.p50,
      "long tasks n/total ms": `${out.longTasks.n}/${out.longTasks.totalMs}`,
      "tile ms hires p50/p95": `${out.tileMs.hires.p50}/${out.tileMs.hires.p95}`,
      "tile ms lowres p50/p95": `${out.tileMs.lowres.p50}/${out.tileMs.lowres.p95}`,
      "heap MB end/peak": `${last?.heapMB}/${out.memory.peakHeapMB}`,
      "geometries/textures end": `${last?.geometries}/${last?.textures}`,
    };
    return out;
  }

  // Resolves once every tile manager the current view renders (hires below distance 1000, every lowres LOD that has
  // tiles) has all its wanted tiles and none loading, for 3 polls in a row, or after timeoutS.
  const untilComplete = (timeoutS = 60) => new Promise((resolve) => {
    let stable = 0;
    const poll = () => {
      const v = viewer(), map = v?.map;
      const managers = map?.isLoaded ? [...(map.lowresTileManager ?? []).filter((m) => m.tiles.size > 0),
        ...(v.controlsManager.distance < 1000 ? [map.hiresTileManager] : [])] : [];
      const counts = managers.map(countTiles);
      const done = managers.length > 0 && counts.every((c) => c.wanted > 0 && c.loading === 0)
        && managers.every((m) => m.currentlyLoading === 0);
      stable = done ? stable + 1 : 0;
      const ms = now();
      if (stable >= 3 || ms > timeoutS * 1000) {
        const h = countTiles(map?.hiresTileManager);
        const l = (map?.lowresTileManager ?? []).map(countTiles);
        return resolve({ ms: round(ms, 0), complete: stable >= 3, hires: h, lowres: l });
      }
      setTimeout(poll, 20);
    };
    poll();
  });

  window.bmPerf = { start, stop, report, untilComplete, sample: () => s && sample() };
})();
