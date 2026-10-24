// Client-side unpacking as the webapp sees it (browser only, run by browser.mjs --pipeline): a packed body goes
// through one of the hand-over strategies below, then through the webapp's RevalidatingFileLoader steps (body
// re-streamed for progress, then arrayBuffer()). Measures wall time for all tiles at once (a view loading) and main
// thread time per tile.

const WORKER = `let x;
onmessage = async ({ data }) => {
  if (data.module) { x = (await WebAssembly.instantiate(data.module)).exports; return; }
  const input = x.bmq3_input(data.body.byteLength);
  new Uint8Array(x.memory.buffer, input, data.body.byteLength).set(new Uint8Array(data.body));
  const length = x.bmq3_unpack();
  const prbm = new Uint8Array(x.memory.buffer, x.bmq3_output(), length).slice();
  postMessage({ id: data.id, prbm }, [prbm.buffer]);
};`;

// the webapp's RevalidatingFileLoader after fetch() resolves
export async function loader(response) {
  if (response.body === undefined || response.body.getReader === undefined) return response.arrayBuffer();
  const reader = response.body.getReader();
  const stream = new ReadableStream({
    start(controller) {
      const next = () =>
        reader.read().then(({ done, value }) => {
          if (done) return controller.close();
          controller.enqueue(value);
          next();
        });
      next();
    },
  });
  return new Response(stream).arrayBuffer();
}

// hands the bytes to arrayBuffer() callers without a copy; no stream body, so the loader skips re-streaming
class Direct extends Response {
  #bytes;
  constructor(bytes, init) {
    super(null, init);
    this.#bytes = bytes;
  }
  get body() {
    return undefined;
  }
  arrayBuffer() {
    return Promise.resolve(this.#bytes.buffer);
  }
}

const streamOf = (bytes) =>
  new ReadableStream({
    start(c) {
      c.enqueue(bytes);
      c.close();
    },
  });

export async function prepare(wasmBytes, bodies, { workers = 0, handover = "bytes" } = {}) {
  const module = await WebAssembly.compile(wasmBytes);
  const x = (await WebAssembly.instantiate(module)).exports;
  let main = 0;
  let unpackOnMain;
  let pool = [];
  if (workers) {
    const url = URL.createObjectURL(new Blob([WORKER], { type: "text/javascript" }));
    const waiting = new Map();
    let nextId = 0;
    pool = Array.from({ length: workers }, () => {
      const w = new Worker(url);
      w.postMessage({ module });
      w.onmessage = ({ data }) => {
        waiting.get(data.id)(data.prbm);
        waiting.delete(data.id);
      };
      return w;
    });
    unpackOnMain = (body) =>
      new Promise((resolve) => {
        const id = nextId++;
        waiting.set(id, resolve);
        const copy = body.slice().buffer;
        pool[id % workers].postMessage({ id, body: copy }, [copy]);
      });
  } else {
    unpackOnMain = async (body) => {
      const input = x.bmq3_input(body.length);
      new Uint8Array(x.memory.buffer, input, body.length).set(body);
      const length = x.bmq3_unpack();
      return new Uint8Array(x.memory.buffer, x.bmq3_output(), length).slice();
    };
  }
  const one = async (tile) => {
    let t = performance.now();
    const pending = unpackOnMain(tile.body);
    main += performance.now() - t;
    const prbm = await pending;
    t = performance.now();
    const init = { headers: { "Content-Type": "application/octet-stream" } };
    const response = handover === "direct" ? new Direct(prbm, init)
      : new Response(handover === "stream" ? streamOf(prbm) : prbm, init);
    main += performance.now() - t;
    const buffer = await loader(response);
    if (buffer.byteLength !== tile.prbm) throw new Error(`${tile.name}: ${buffer.byteLength} bytes`);
  };
  // all tiles at once, as a view requests them; the first pass is the cold one (tier-up, memory growth)
  const pass = async () => {
    main = 0;
    const start = performance.now();
    await Promise.all(bodies.map(one));
    const per = (ms) => +(ms / bodies.length).toFixed(3);
    return { wallMsPerTile: per(performance.now() - start), ourMainMsPerTile: per(main) };
  };
  return { pass, close: () => pool.forEach((w) => w.terminate()), isolated: crossOriginIsolated };
}
