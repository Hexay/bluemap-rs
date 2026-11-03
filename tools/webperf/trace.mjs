// Chrome trace (Tracing domain) summary: busy ms per thread and top self-time events of the page's main thread, so
// native time the JS profiler only shows as "(program)" gets a name.
// v8/blink/cc are left out: at their volume a 10 s capture outgrows the session's memory and Chrome is killed
export const TRACE_CATEGORIES = ["toplevel", "devtools.timeline", "disabled-by-default-devtools.timeline", "gpu",
  "viz"].join(",");

// Begins tracing; returns stop() resolving to the summary. Events arrive through the connect() onEvent hook.
export async function startTrace(send, events) {
  events.length = 0;
  await send("Tracing.start", { categories: TRACE_CATEGORIES, transferMode: "ReportEvents" });
  return async () => {
    const done = new Promise((resolve) => (events.complete = resolve));
    await send("Tracing.end");
    await done;
    return summarizeTrace(events);
  };
}

// Feed from onEvent: (method, params) => traceEvent(events, method, params)
export function traceEvent(events, method, params) {
  if (method === "Tracing.dataCollected") events.push(...params.value);
  else if (method === "Tracing.tracingComplete") events.complete?.();
}

function selfTimes(xs) {
  // complete events of one thread, nested by time: self = duration minus direct children
  xs.sort((a, b) => a.ts - b.ts || b.dur - a.dur);
  const self = new Map();
  const stack = [];
  let busy = 0;
  for (const e of xs) {
    while (stack.length && stack[stack.length - 1].end <= e.ts) stack.pop();
    const parent = stack[stack.length - 1];
    if (parent) parent.child += e.dur;
    else busy += e.dur;
    const node = { e, end: e.ts + e.dur, child: 0 };
    stack.push(node);
    e.node = node;
  }
  for (const e of xs) self.set(e.name, (self.get(e.name) ?? 0) + e.dur - e.node.child);
  return { busy, self };
}

export function summarizeTrace(events) {
  const names = new Map();
  for (const e of events) if (e.ph === "M" && e.name === "thread_name") names.set(`${e.pid}:${e.tid}`, e.args.name);
  const byThread = new Map();
  for (const e of events) {
    if (e.ph !== "X" || !e.dur) continue;
    const k = `${e.pid}:${e.tid}`;
    if (!byThread.has(k)) byThread.set(k, []);
    byThread.get(k).push(e);
  }
  const threads = [];
  let main = null;
  for (const [k, xs] of byThread) {
    const { busy, self } = selfTimes(xs);
    const name = names.get(k) ?? k;
    const t = { name, busyMs: Math.round(busy / 1000), self };
    threads.push(t);
    if (name === "CrRendererMain" && (!main || t.busyMs > main.busyMs)) main = t;
  }
  const merged = new Map();
  for (const t of threads) merged.set(t.name, (merged.get(t.name) ?? 0) + t.busyMs);
  return {
    threadsBusyMs: Object.fromEntries([...merged.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10)),
    mainTop: main ? [...main.self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 15)
      .map(([name, us]) => ({ name, ms: Math.round(us / 1000) })) : [],
  };
}
