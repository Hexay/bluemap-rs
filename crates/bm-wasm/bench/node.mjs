// usage: node crates/bm-wasm/bench/node.mjs <corpus dir> [unpack.wasm=crates/bm-web/client/unpack.wasm] [reps=5]
//   [--frames]
// WASM unpack time per tile in Node (see core.mjs); prints one JSON line. --frames: from the zstd frames.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { run } from "./core.mjs";

const args = process.argv.slice(2);
const frames = args.includes("--frames");
const [dir, wasm = "crates/bm-web/client/unpack.wasm", reps = "5"] = args.filter((a) => a !== "--frames");
const manifest = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8"));
const file = (t) => (frames ? t.name.replace(/\.body$/, ".frame") : t.name);
const bodies = manifest.map((t) => ({ ...t, body: new Uint8Array(readFileSync(join(dir, file(t)))) }));
console.log(JSON.stringify({ wasm, frames, ...(await run(readFileSync(wasm), bodies, Number(reps), frames)) }));
