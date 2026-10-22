// bluemap-rs client-side unpacking (docs/18). Hires tiles of an optimized storage are stored packed; this asks the
// server for them as stored and unpacks them here, so the server neither unpacks nor recompresses them.
// Loaded ahead of the webapp, whose tile requests go through fetch(). Anything unexpected falls back to plain PRBM.
// Unpacking runs in workers when the page may start them, else on the main thread (docs/24).
// A viewer turns it off with: localStorage.setItem("bluemap-rs-client-unpack", "off")
(() => {
  "use strict";
  const MEDIA_TYPE = "application/vnd.bluemap.bmq3";
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

  // body → PRBM in a buffer of its own; runs on the main thread and, stringified, in each worker
  function decoder(module) {
    // a module's memory never shrinks: one grown past this by a huge tile is dropped after it
    const MEMORY_LIMIT = 64 << 20;
    let exports = null;
    return async (body) => {
      exports ??= (await WebAssembly.instantiate(module)).exports;
      const x = exports;
      const input = x.bmq3_input(body.length);
      new Uint8Array(x.memory.buffer, input, body.length).set(body);
      const length = x.bmq3_unpack();
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
    const prbm = await unpack(new Uint8Array(data.body));
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
    let pool;
    try {
      const url = URL.createObjectURL(new Blob([WORKER], { type: "text/javascript" }));
      pool = Array.from({ length: workerCount() }, () => ({ worker: new Worker(url), jobs: new Map() }));
    } catch {
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
      entry.worker.postMessage({ module });
    }
    return (body) => {
      const live = pool.filter((e) => !e.dead);
      if (!live.length) return decoder(module)(body);
      const entry = live.reduce((a, b) => (b.jobs.size < a.jobs.size ? b : a));
      const id = nextId++;
      return new Promise((resolve, reject) => {
        entry.jobs.set(id, { resolve, reject });
        entry.worker.postMessage({ id, body: body.buffer }, [body.buffer]);
      });
    };
  }

  let unpack = null;
  const ready = WebAssembly.compile(Uint8Array.from(atob(WASM), (c) => c.charCodeAt(0))).then(
    (module) => {
      unpack = workerPool(module) ?? decoder(module);
    },
    () => {},
  );

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

  scope.fetch = async function (input, init) {
    let url;
    try {
      url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, scope.location.href);
    } catch {
      return plainFetch.call(this, input, init);
    }
    if (!isHiresTile(url)) return plainFetch.call(this, input, init);
    await ready;
    if (!unpack) return plainFetch.call(this, input, init);

    const request = new Request(input, init);
    request.headers.set("Accept", `${MEDIA_TYPE}, */*`);
    const response = await plainFetch.call(this, request);
    if (!(response.headers.get("Content-Type") || "").startsWith(MEDIA_TYPE)) return response;
    let prbm;
    try {
      prbm = await unpack(new Uint8Array(await response.arrayBuffer()));
    } catch (error) {
      console.warn("bluemap-rs: unpacking a tile failed, loading it unpacked", error);
      return plainFetch.call(this, input, init);
    }
    const headers = new Headers(response.headers);
    headers.set("Content-Type", "application/octet-stream");
    headers.set("Content-Length", String(prbm.length));
    headers.delete("Content-Encoding");
    return new Unpacked(prbm, { status: response.status, statusText: response.statusText, headers });
  };
})();
