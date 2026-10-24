// The shipped client script on the corpus (browser only, run by browser.mjs --client): fetch() answers tile URLs
// with packed bodies as the server would, the script under test wraps it, and each tile then goes through the
// webapp's RevalidatingFileLoader steps, at most 8 at a time like its TileManager. Checks every PRBM.
import { loader } from "./pipeline.mjs";

const MEDIA_TYPE = "application/vnd.bluemap.bmq3";
const fnv = (bytes) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < bytes.length; i++) h = Math.imul(h ^ bytes[i], 0x01000193);
  return h >>> 0;
};

export async function prepare(script, corpus, { concurrency = 8, tiles = corpus.length } = {}) {
  const bodies = corpus.slice(0, tiles);
  const tileUrl = (i) => `${location.origin}/maps/m/tiles/0/x${i}/z0.prbm`;
  globalThis.fetch = async (input) => {
    const url = typeof input === "string" ? input : input.url;
    const i = Number(/\/x(\d+)\/z0\.prbm$/.exec(url)?.[1]);
    return new Response(bodies[i].body.slice(), { headers: { "Content-Type": MEDIA_TYPE } });
  };
  (0, eval)(script);
  const one = async (i, check) => {
    const prbm = new Uint8Array(await loader(await fetch(tileUrl(i))));
    if (check && (prbm.length !== bodies[i].prbm || fnv(prbm) !== bodies[i].fnv)) throw new Error(`tile ${i} differs`);
  };
  // the first pass is the cold one (module compile, workers starting, tier-up); browser.mjs ends with a checked one
  const pass = async (check = false) => {
    const start = performance.now();
    let next = 0;
    const lane = async () => {
      while (next < bodies.length) await one(next++, check);
    };
    await Promise.all(Array.from({ length: concurrency }, lane));
    return { wallMsPerTile: +((performance.now() - start) / bodies.length).toFixed(3) };
  };
  return { pass, tiles: bodies.length };
}
