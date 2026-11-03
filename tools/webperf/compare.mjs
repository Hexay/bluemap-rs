// usage: node tools/webperf/compare.mjs <results.jsonl…> [--rows "view ms,lowres tile ms p50"] [--markdown]
// Medians per label/scenario column over the runs run.mjs appended with --out.
import { readFileSync } from "node:fs";
import { medianTable } from "./rows.mjs";

const args = process.argv.slice(2);
const at = args.indexOf("--rows");
const only = at >= 0 ? args.splice(at, 2)[1].split(",") : null;
const markdown = args.includes("--markdown") && args.splice(args.indexOf("--markdown"), 1);
const results = args.flatMap((f) => readFileSync(f, "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l)));
const count = new Map();
results.forEach((r) => count.set(`${r.label}/${r.scenario}`, (count.get(`${r.label}/${r.scenario}`) ?? 0) + 1));
const table = medianTable(results, (r) => `${r.label}/${r.scenario}`);
const rows = Object.entries(table).filter(([k]) => !only || only.includes(k));
if (!markdown) {
  console.table(Object.fromEntries(rows));
} else {
  const cols = Object.keys(rows[0][1]);
  console.log(`| median (runs) | ${cols.map((c) => `${c} (${count.get(c)})`).join(" | ")} |`);
  console.log(`|---|${cols.map(() => "---:").join("|")}|`);
  rows.forEach(([k, v]) => console.log(`| ${k} | ${cols.map((c) => v[c] ?? "–").join(" | ")} |`));
}
