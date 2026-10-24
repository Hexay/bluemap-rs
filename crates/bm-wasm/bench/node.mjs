// usage: node crates/bm-wasm/bench/node.mjs <corpus dir> [unpack.wasm=crates/bm-web/client/unpack.wasm] [reps=5]
// WASM unpack time per tile in Node (see core.mjs); prints one JSON line.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { run } from "./core.mjs";

const [dir, wasm = "crates/bm-web/client/unpack.wasm", reps = "5"] = process.argv.slice(2);
const manifest = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8"));
const bodies = manifest.map((t) => ({ ...t, body: new Uint8Array(readFileSync(join(dir, t.name))) }));
console.log(JSON.stringify({ wasm, ...(await run(readFileSync(wasm), bodies, Number(reps))) }));
