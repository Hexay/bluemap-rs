// The unpack benchmark shared by node.mjs and browser.mjs: times unpack.wasm on a corpus from
// `cargo run --release -p bm-wasm --example corpus`, the way client/unpack.js calls it.

const fnv = (bytes) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < bytes.length; i++) h = Math.imul(h ^ bytes[i], 0x01000193);
  return h >>> 0;
};

// bodies: [{ name, quads, prbm, fnv, body: Uint8Array }]; returns per-phase totals in ms
export async function run(wasmBytes, bodies, reps = 5) {
  const t0 = performance.now();
  const { instance } = await WebAssembly.instantiate(wasmBytes);
  const instantiateMs = performance.now() - t0;
  const x = instance.exports;
  const memory = () => x.memory.buffer.byteLength;
  const initialMemory = memory();
  const unpack = (body) => {
    const input = x.bmq3_input(body.length);
    new Uint8Array(x.memory.buffer, input, body.length).set(body);
    const length = x.bmq3_unpack();
    if (length < 0) throw new Error("corrupt");
    return new Uint8Array(x.memory.buffer, x.bmq3_output(), length);
  };

  // cold: the first pass over the corpus in file order, as a viewer's first tiles would be
  let cold = 0;
  for (const tile of bodies) {
    const start = performance.now();
    const view = unpack(tile.body);
    view.slice();
    cold += performance.now() - start;
    if (view.length !== tile.prbm || fnv(view) !== tile.fnv) throw new Error(`${tile.name} differs`);
  }
  const peakMemory = memory();

  // whole passes, fastest of `reps`: browsers coarsen performance.now() to 100 µs outside cross-origin isolation
  const pass = (copyOut) => {
    let best = Infinity;
    for (let r = 0; r < reps; r++) {
      const start = performance.now();
      for (const tile of bodies) {
        const view = unpack(tile.body);
        if (copyOut) view.slice();
      }
      best = Math.min(best, performance.now() - start);
    }
    return best;
  };
  const unpackOnly = pass(false);
  const totals = { unpack: unpackOnly, copyOut: pass(true) - unpackOnly };
  const quads = bodies.reduce((n, t) => n + t.quads, 0);
  const prbm = bodies.reduce((n, t) => n + t.prbm, 0);
  const ns = (ms) => +((ms * 1e6) / quads).toFixed(1);
  const perTile = (ms) => +(ms / bodies.length).toFixed(3);
  return {
    tiles: bodies.length,
    quads,
    prbmMB: +(prbm / 1e6).toFixed(1),
    instantiateMs: +instantiateMs.toFixed(2),
    coldMsPerTile: perTile(cold),
    unpackNsPerQuad: ns(totals.unpack),
    unpackMsPerTile: perTile(totals.unpack),
    copyOutMsPerTile: perTile(totals.copyOut),
    memoryMB: { initial: +(initialMemory / 2 ** 20).toFixed(1), peak: +(peakMemory / 2 ** 20).toFixed(1) },
  };
}
