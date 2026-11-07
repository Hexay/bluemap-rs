// bluemap-rs client-side unpacking (docs/18). Hires tiles of an optimized storage are stored packed; this asks the
// server for them as stored and unpacks them here, so the server neither unpacks nor recompresses them.
// Loaded ahead of the webapp, whose tile requests go through fetch(). Anything unexpected falls back to plain PRBM.
// Unpacking runs in workers when the page may start them, else on the main thread (docs/24).
// A viewer turns it off with: localStorage.setItem("bluemap-rs-client-unpack", "off")
(() => {
  "use strict";
  const MEDIA_TYPE = "application/vnd.bluemap.bmq3";
  // the body's zstd frame, for a browser that does not take zstd as a content coding
  const FRAME_TYPE = "application/vnd.bluemap.bmq3+zstd";
  const SWITCH = "bluemap-rs-client-unpack";
  const WASM = "__UNPACK_WASM_BASE64__";
  const scope = globalThis;
  const plainFetch = scope.fetch;

  let off = false;
  try {
    off = scope.localStorage.getItem(SWITCH) === "off";
  } catch {
    // storage blocked: keep the default
  }
  if (off || typeof plainFetch !== "function" || typeof WebAssembly !== "object") return;

  // features measured one by one (docs/18); localStorage "bluemap-rs-client-tuning" overrides them for an A/B
  const TUNING = { ahead: 1, slots: 1, markerBatch: 64, markerPoll: 1 };
  try {
    Object.assign(TUNING, JSON.parse(scope.localStorage.getItem("bluemap-rs-client-tuning")));
  } catch {
    // storage blocked or nothing valid in it: the defaults
  }
  const hiresLoading = new Set();
  const hiresDirty = new Set();

  // body (or its zstd frame) → PRBM in a buffer of its own; runs on the main thread and, stringified, in each worker
  function decoder(module) {
    // a module's memory never shrinks: one grown past this by a huge tile is dropped after it
    const MEMORY_LIMIT = 64 << 20;
    let exports = null;
    return async (body, frame) => {
      exports ??= (await WebAssembly.instantiate(module)).exports;
      const x = exports;
      const input = x.bmq3_input(body.length);
      new Uint8Array(x.memory.buffer, input, body.length).set(body);
      const length = frame ? x.bmq3_unpack_frame() : x.bmq3_unpack();
      if (length < 0) throw new Error("corrupt packed tile");
      const prbm = new Uint8Array(x.memory.buffer, x.bmq3_output(), length).slice();
      if (x.memory.buffer.byteLength > MEMORY_LIMIT) exports = null;
      return prbm;
    };
  }

  const WORKER = `"use strict";
const decoder = ${decoder};
let unpack;
onmessage = async ({ data }) => {
  if (data.module) return void (unpack = decoder(data.module));
  try {
    const prbm = await unpack(new Uint8Array(data.body), data.frame);
    postMessage({ id: data.id, prbm }, [prbm.buffer]);
  } catch (error) {
    postMessage({ id: data.id, error: String(error) });
  }
};`;

  // the webapp keeps up to 8 tiles loading; more workers than half of that sit idle
  const workerCount = () => Math.min(4, Math.max(1, (scope.navigator?.hardwareConcurrency || 2) - 2));

  // an unpack function backed by workers, or null where workers cannot start (no Worker, CSP without blob:)
  function workerPool(module) {
    if (typeof Worker !== "function") return null;
    const pool = [];
    try {
      const url = URL.createObjectURL(new Blob([WORKER], { type: "text/javascript" }));
      for (let i = 0; i < workerCount(); i++) pool.push({ worker: new Worker(url), jobs: new Map() });
      // throws where a compiled module cannot be sent to a worker
      pool.forEach((entry) => entry.worker.postMessage({ module }));
    } catch {
      pool.forEach((entry) => entry.worker.terminate());
      return null;
    }
    let nextId = 0;
    for (const entry of pool) {
      entry.worker.onmessage = ({ data }) => {
        const job = entry.jobs.get(data.id);
        entry.jobs.delete(data.id);
        data.error ? job.reject(new Error(data.error)) : job.resolve(data.prbm);
      };
      // a worker that fails to start (CSP blocks blob: workers) reports it here, asynchronously
      entry.worker.onerror = (event) => {
        event.preventDefault();
        entry.dead = true;
        entry.jobs.forEach((job) => job.reject(new Error("unpack worker failed")));
        entry.jobs.clear();
        if (pool.every((e) => e.dead)) unpack = decoder(module);
      };
    }
    return (body, frame) => {
      const live = pool.filter((e) => !e.dead);
      if (!live.length) return decoder(module)(body, frame);
      const entry = live.reduce((a, b) => (b.jobs.size < a.jobs.size ? b : a));
      const id = nextId++;
      return new Promise((resolve, reject) => {
        entry.jobs.set(id, { resolve, reject });
        entry.worker.postMessage({ id, body: body.buffer, frame }, [body.buffer]);
      });
    };
  }

  let unpack = null;
  const ready = WebAssembly.compile(Uint8Array.from(atob(WASM), (c) => c.charCodeAt(0)))
    .then((module) => {
      unpack = workerPool(module) ?? decoder(module);
    })
    .catch(() => {});

  // Hands the PRBM to arrayBuffer() as is. Without a body stream the webapp's loader reads arrayBuffer() directly
  // instead of re-streaming 1–2 MB through a second Response on the main thread.
  class Unpacked extends Response {
    #prbm;
    constructor(prbm, init) {
      super(null, init);
      this.#prbm = prbm;
    }
    get body() {
      return undefined;
    }
    arrayBuffer() {
      return Promise.resolve(this.#prbm.buffer);
    }
    bytes() {
      return Promise.resolve(this.#prbm);
    }
    blob() {
      return Promise.resolve(new Blob([this.#prbm], { type: "application/octet-stream" }));
    }
    text() {
      return Promise.resolve(new TextDecoder().decode(this.#prbm));
    }
    clone() {
      return new Unpacked(this.#prbm.slice(), this);
    }
  }

  // maps/<id>/tiles/0/x…/z….prbm
  const isHiresTile = (url) => /\/tiles\/0\/x[-\d/]+z[-\d/]+\.prbm$/.test(url.pathname);
  const isLowresTile = (url) => /\/tiles\/[1-9]\d*\/x[-\d/]+z[-\d/]+\.png$/.test(url.pathname);

// __CLIENT_PARTS__

  scope.fetch = async function (input, init) {
    let url;
    try {
      url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, scope.location.href);
    } catch {
      return plainFetch.call(this, input, init);
    }
    if (TUNING.markerPoll && isLiveMarkers(url)) return fetchLiveMarkers(this, input, init);
    if (isLowresTile(url)) {
      const request = new Request(input, init);
      return (await revalidated(request, url)) ?? plainFetch.call(this, request);
    }
    if (!isHiresTile(url)) return plainFetch.call(this, input, init);
    await ready;
    if (!unpack) return plainFetch.call(this, input, init);

    const request = new Request(input, init);
    request.headers.set("Accept", `${FRAME_TYPE}, ${MEDIA_TYPE}, */*`);
    const m = TILE_PATH.exec(url.pathname);
    const [x, z] = [m[3], m[4]].map((s) => Number(s.replaceAll("/", "")));
    const key = tileKey(x, z);
    try {
      noteHiresRequest(url, m[1], x, z);
    } catch {
      // prefetching is optional
    }
    hiresLoading.add(key);
    try {
      const response = (await claimHeld(x, z, url, request)) ?? (await revalidated(request, url)) ?? (await plainFetch.call(this, request));
      const type = (response.headers.get("Content-Type") || "").split(";")[0].trim().toLowerCase();
      if (type !== MEDIA_TYPE && type !== FRAME_TYPE) return response;
      let prbm;
      try {
        prbm = await unpack(new Uint8Array(await response.arrayBuffer()), type === FRAME_TYPE);
      } catch (error) {
        console.warn("bluemap-rs: unpacking a tile failed, loading it unpacked", error);
        return plainFetch.call(this, input, init);
      }
      if (hiresDirty.delete(key)) return scope.fetch(new Request(new Request(input, init), { cache: "no-cache" }));
      const headers = new Headers(response.headers);
      headers.set("Content-Type", "application/octet-stream");
      headers.set("Content-Length", String(prbm.length));
      headers.delete("Content-Encoding");
      return new Unpacked(prbm, { status: response.status, statusText: response.statusText, headers });
    } finally {
      hiresLoading.delete(key);
      hiresDirty.delete(key);
    }
  };

  // the webapp reloads a tile on these; any answer held for that tile is out of date first
  const PlainEventSource = scope.EventSource;
  if (typeof PlainEventSource === "function") {
    scope.EventSource = class extends PlainEventSource {
      constructor(...args) {
        super(...args);
        this.addEventListener("tile", ({ data }) => {
          try {
            const t = JSON.parse(data);
            tileChanged(t.lod, t.x, t.y);
          } catch {
            // the webapp's own listener decides what to do with it
          }
        });
      }
    };
  }
})();
