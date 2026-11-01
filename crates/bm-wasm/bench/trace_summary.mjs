// usage: node crates/bm-wasm/bench/trace_summary.mjs <trace.json>   (re-summarizes what gpu_trace.mjs saved)
// Main-thread time of the page's renderer, by self time of nested trace events; script self time is split further by
// the V8 sampling profiler's stacks, since the trace has no event around tile parsing.
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const PARSE_COMPILE = /^(v8\.compile|v8\.parseOnBackground|v8\.compileModule|v8\.produce|V8\.(Compile|Parse|Deserialize)|v8\.wasm\.|wasm\.|ParseHTML|HTMLDocumentParser|ParseAuthorStyleSheet|BackgroundParse)/;
const SCRIPT = /^(FunctionCall|EvaluateScript|v8\.run|v8\.callFunction|v8\.evaluateModule|RunMicrotasks|TimerFire|FireAnimationFrame|EventDispatch|XHR|V8\.Execute|v8\.execute|RunMicrotask|FireIdleCallback|ResourceReceivedData)/;
const GL_EVENT = /^(GLES2|GL|WebGL|CommandBuffer|ImplementationBase|GpuChannel|gpu::|SharedImage|ContextProvider|DrawingBuffer)/;
// the WebGL entry points as the sampling profiler names them
const GL_FUNCTION = /^(drawElements|drawArrays|drawElementsInstanced|drawArraysInstanced|bufferData|bufferSubData|texImage2D|texImage3D|texSubImage2D|texSubImage3D|compressedTex|uniform|vertexAttrib|enableVertexAttribArray|disableVertexAttribArray|bindBuffer|bindTexture|bindVertexArray|bindFramebuffer|useProgram|createBuffer|createTexture|createVertexArray|deleteBuffer|deleteTexture|linkProgram|compileShader|shaderSource|attachShader|getProgramParameter|getShaderParameter|getProgramInfoLog|getShaderInfoLog|getActiveUniform|getActiveAttrib|getUniformLocation|getAttribLocation|activeTexture|pixelStorei|texParameter|clear|viewport|scissor|enable$|disable$|blendFunc|blendEquation|depthFunc|depthMask|colorMask|cullFace|frontFace|flush|finish|getError|getParameter|getExtension|readPixels|invalidateFramebuffer|generateMipmap)/;
const TILE_PARSE = /^(decodePrwm|read4ByteInt|decodePrbm)$/;

const threadsOf = (events) => {
  const names = new Map();
  for (const e of events) if (e.ph === "M" && e.name === "thread_name") names.set(`${e.pid}:${e.tid}`, e.args.name);
  const threads = new Map();
  for (const e of events) {
    if (e.ph !== "X" || e.dur === undefined) continue;
    const key = `${e.pid}:${e.tid}`;
    if (!threads.has(key)) threads.set(key, { key, name: names.get(key) ?? "?", events: [] });
    threads.get(key).events.push(e);
  }
  return [...threads.values()];
};

// walks nested complete events; visit(event, selfUs, depth, ancestors); returns the roots
function walk(thread, visit) {
  const sorted = thread.events.toSorted((a, b) => a.ts - b.ts || b.dur - a.dur);
  const stack = [];
  const roots = [];
  const close = (until) => {
    while (stack.length && stack.at(-1).end <= until) {
      const top = stack.pop();
      visit(top.event, Math.max(0, top.event.dur - top.children), stack.map((f) => f.event));
    }
  };
  for (const e of sorted) {
    close(e.ts);
    if (stack.length) stack.at(-1).children += e.dur;
    else roots.push(e);
    stack.push({ event: e, end: e.ts + e.dur, children: 0 });
  }
  close(Infinity);
  return roots;
}

const busy = (thread) => (thread ? walk(thread, () => {}).reduce((n, e) => n + e.dur, 0) / 1e3 : 0);

// the sampling profiler's self time on one thread, split by what the stack is doing: shares of the samples
function sampleShares(events, thread) {
  const nodes = new Map();
  let total = 0, tile = 0, gl = 0, sampled = false;
  const glNames = new Map();
  // gotcha: a chunk carries the sampler's tid; only its Profile event names the sampled thread, joined by pid + id
  const profiles = new Set(events.filter((e) => e.name === "Profile" && `${e.pid}:${e.tid}` === thread.key).map((e) => `${e.pid}:${e.id}`));
  const chunks = events.filter((e) => e.name === "ProfileChunk" && profiles.has(`${e.pid}:${e.id}`));
  for (const c of chunks) {
    for (const n of c.args.data?.cpuProfile?.nodes ?? []) nodes.set(n.id, n);
  }
  // decodePrwm is usually inlined into PRBMLoader's `parse`, the only hot `parse` of the bundle during a load
  const isTileFrame = (f) => TILE_PARSE.test(f.functionName) || (f.functionName === "parse" && /index-[^/]*\.js/.test(f.url ?? ""));
  for (const c of chunks) {
    const samples = c.args.data?.cpuProfile?.samples ?? [];
    const deltas = c.args.data?.timeDeltas ?? [];
    samples.forEach((id, i) => {
      const leaf = nodes.get(id);
      if (!leaf) return;
      const dt = Math.max(0, deltas[i] ?? 0);
      const name = leaf.callFrame.functionName;
      if (["(idle)", "(program)", "(root)"].includes(name)) return;
      sampled = true;
      total += dt;
      let isTile = false;
      for (let n = leaf; n; n = nodes.get(n.parent)) {
        if (isTileFrame(n.callFrame)) { isTile = true; break; }
      }
      if (isTile) tile += dt;
      else if (GL_FUNCTION.test(name) && !leaf.callFrame.url) {
        gl += dt;
        glNames.set(name, (glNames.get(name) ?? 0) + dt);
      }
    });
  }
  const top = [...glNames].sort((a, b) => b[1] - a[1]).slice(0, 5).map(([n, t]) => `${n} ${Math.round(t / 1e3)}`);
  return sampled ? { tile: tile / total, gl: gl / total, glTop: top, sampledMs: total / 1e3 } : null;
}

export function summarize(trace) {
  const events = trace.traceEvents ?? trace;
  const threads = threadsOf(events);
  const mains = threads.filter((t) => t.name === "CrRendererMain");
  const main = mains.toSorted((a, b) => busy(b) - busy(a))[0];
  if (!main) return { error: "no CrRendererMain thread in the trace" };
  const buckets = { parseCompile: 0, script: 0, glEvents: 0, other: 0 };
  const otherNames = new Map();
  const classify = (e, ancestors) => {
    if (GL_EVENT.test(e.name)) return "glEvents";
    if (PARSE_COMPILE.test(e.name)) return "parseCompile";
    if (SCRIPT.test(e.name)) return "script";
    // V8 internals and bindings count as what they run under
    for (let i = ancestors.length - 1; i >= 0; i--) {
      const a = ancestors[i].name;
      if (GL_EVENT.test(a)) return "glEvents";
      if (PARSE_COMPILE.test(a)) return "parseCompile";
      if (SCRIPT.test(a)) return "script";
    }
    return "other";
  };
  const roots = walk(main, (e, self, ancestors) => {
    const b = classify(e, ancestors);
    buckets[b] += self / 1e3;
    if (b === "other") otherNames.set(e.name, (otherNames.get(e.name) ?? 0) + self / 1e3);
  });
  const t0 = Math.min(...roots.map((e) => e.ts));
  const shares = sampleShares(events, main);
  const tileMs = shares ? buckets.script * shares.tile : null;
  const glSampledMs = shares ? buckets.script * shares.gl : 0;
  const children = new Map();
  walk(main, (e, _self, ancestors) => {
    if (ancestors[0]) {
      const m = children.get(ancestors[0]) ?? new Map();
      m.set(e.name, (m.get(e.name) ?? 0) + e.dur / 1e3);
      children.set(ancestors[0], m);
    }
  });
  const longest = roots.toSorted((a, b) => b.dur - a.dur).slice(0, 5).map((e) => ({
    atMs: Math.round((e.ts - t0) / 1e3), ms: Math.round(e.dur / 1e3),
    inside: [...(children.get(e) ?? [])].sort((a, b) => b[1] - a[1]).slice(1, 4).map(([n, t]) => `${n} ${Math.round(t)}`),
  }));
  const sum = (names) => threads.filter((t) => names.test(t.name)).reduce((n, t) => n + busy(t), 0);
  return {
    mainMs: busy(main), rootsSum: roots.length,
    split: {
      script: buckets.script - (tileMs ?? 0) - glSampledMs, parseCompile: buckets.parseCompile, tileParsing: tileMs,
      glCalls: buckets.glEvents + glSampledMs, other: buckets.other,
    },
    glEventsMs: buckets.glEvents, glTop: shares?.glTop ?? [], sampledMs: shares?.sampledMs ?? 0,
    otherTop: [...otherNames].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([n, t]) => `${n} ${Math.round(t)}`),
    longest,
    gpuMainMs: sum(/^CrGpuMain$/), vizMs: sum(/^VizCompositorThread$/), workerMs: sum(/^DedicatedWorker/),
    compositorMs: sum(/^Compositor$/), rasterMs: sum(/^CompositorTileWorker/), threads: threads.length,
  };
}

const ms = (x) => (x === null || x === undefined ? "n/a" : Math.round(x));
export function printSummary(name, view, s, path) {
  console.log(`\n== ${name}: time to view ${ms(view?.ms)} ms (${view?.loaded}/${view?.wanted} tiles), page longtask max ${ms(view?.longest)} ms`);
  if (s.error) return console.log(s.error);
  const p = s.split;
  console.log(`main thread ${ms(s.mainMs)} ms: script ${ms(p.script)} | parse-compile ${ms(p.parseCompile)} | tile parsing ${ms(p.tileParsing)} | GL calls ${ms(p.glCalls)} | other ${ms(p.other)}`);
  console.log(`  GL: trace events ${ms(s.glEventsMs)}, sampled top: ${s.glTop.join(", ") || "-"}; profiler saw ${ms(s.sampledMs)} ms of JS`);
  console.log(`  other: ${s.otherTop.join(", ")}`);
  console.log(`longest tasks: ${s.longest.map((t) => `${t.ms} ms at ${t.atMs} [${t.inside.join(", ")}]`).join("; ")}`);
  console.log(`GPU process main ${ms(s.gpuMainMs)} ms, viz compositor ${ms(s.vizMs)} ms; renderer compositor ${ms(s.compositorMs)} ms, raster ${ms(s.rasterMs)} ms; workers (unpack) ${ms(s.workerMs)} ms`);
  if (path) console.log(`trace: ${path}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  printSummary(process.argv[2], null, summarize(JSON.parse(readFileSync(process.argv[2], "utf8"))));
}
