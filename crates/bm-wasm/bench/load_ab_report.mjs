// usage: node crates/bm-wasm/bench/load_ab_report.mjs rows.jsonl     medians of load_ab.mjs rows, per net and label
import { readFileSync } from "node:fs";

const rows = readFileSync(process.argv[2], "utf8").trim().split("\n").map((l) => JSON.parse(l));
const median = (xs) => {
  const s = xs.filter((x) => x !== null && x !== undefined).toSorted((a, b) => a - b);
  return s.length ? s[s.length >> 1] : null;
};
const cell = (xs) => {
  const s = xs.filter((x) => x !== null && x !== undefined);
  return s.length ? `${Math.round(median(s))} (${Math.round(Math.min(...s))}-${Math.round(Math.max(...s))})` : "-";
};
const voided = rows.filter((r) => !r.valid);
if (voided.length) console.log(`voided: ${voided.map((r) => `${r.label}/${r.net}/${r.run}: ${r.why}`).join("; ")}`);
const valid = rows.filter((r) => r.valid);
const sum = (o, f) => Object.values(o ?? {}).reduce((n, c) => n + (f(c) ?? 0), 0);

for (const net of new Set(valid.map((r) => r.net))) {
  const labels = [...new Set(valid.filter((r) => r.net === net).map((r) => r.label))];
  const of = (l) => valid.filter((r) => r.net === net && r.label === l);
  console.log(`\n## ${net}\n\n| row | n | phase | view ms | on-screen ms | tile p50 ms | main ms | requests | KB | >33 | >50 | longest | empty/wanted |`);
  console.log("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|");
  for (const l of labels) {
    for (const ph of ["first", "reload", "revisit"]) {
      const ps = of(l).map((r) => r.phases[ph]).filter(Boolean);
      if (!ps.length) continue;
      console.log(`| ${l} | ${ps.length} | ${ph} | ${cell(ps.map((p) => p.ms))} | ${cell(ps.map((p) => p.onscreenMs))} | ${cell(ps.map((p) => p.tileP50Ms))} | ${cell(ps.map((p) => p.mainMs))} `
        + `| ${cell(ps.map((p) => sum(p.total, (c) => c.n)))} | ${cell(ps.map((p) => sum(p.total, (c) => c.kb)))} | ${cell(ps.map((p) => p.frames.over33))} | ${cell(ps.map((p) => p.frames.over50))} `
        + `| ${cell(ps.map((p) => p.frames.longest))} | ${Math.max(...ps.map((p) => p.holes.hiresEmpty))}/${median(ps.map((p) => p.holes.hires))} |`);
    }
  }
  console.log("\n| row | pan tile p50 | p95 | max | tiles | never | hires requests | hires KB | KB all | >33 | >50 | main ms | empty |");
  console.log("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
  for (const l of labels) {
    const ps = of(l).map((r) => r.phases.pan).filter(Boolean);
    if (!ps.length) continue;
    console.log(`| ${l} | ${cell(ps.map((p) => p.tileLatency.p50))} | ${cell(ps.map((p) => p.tileLatency.p95))} | ${cell(ps.map((p) => p.tileLatency.max))} | ${cell(ps.map((p) => p.tileLatency.tiles))} `
      + `| ${cell(ps.map((p) => p.never))} | ${cell(ps.map((p) => p.total.hires?.n))} | ${cell(ps.map((p) => p.total.hires?.kb))} | ${cell(ps.map((p) => sum(p.total, (c) => c.kb)))} `
      + `| ${cell(ps.map((p) => p.frames.over33))} | ${cell(ps.map((p) => p.frames.over50))} | ${cell(ps.map((p) => p.mainMs))} | ${Math.max(...ps.map((p) => p.holes.hiresEmpty))} |`);
  }
  for (const l of labels) {
    const traced = of(l).filter((r) => r.phases.reload?.partition);
    if (!traced.length) continue;
    const p = (f) => cell(traced.map((r) => f(r.phases.reload)));
    console.log(`\n### ${l}: traced reload, ${traced.length} runs (traced times run slower)\n`);
    console.log(`view ${p((x) => x.ms)}; partition ${["mainBusy", "networkWait", "workersOnly", "idle", "mainAndNetwork", "workerBusyMs", "gpuBusyMs", "hiresWallMs"].map((k) => `${k} ${p((x) => x.partition[k])}`).join("; ")}`);
    console.log(`main split: ${["script", "parseCompile", "tileParsing", "glCalls", "other"].map((k) => `${k} ${p((x) => x.mainSplit[k])}`).join("; ")}`);
    console.log(`milestones: ${["firstTile", "firstHires", "lastHiresStart", "lastHiresEnd", "lastEnd"].map((k) => `${k} ${p((x) => x.milestones[k])}`).join("; ")}`);
    console.log("\n| category (until view) | n | conditional | statuses (run 1) | KB | queue ms med | queue sum | RTT ms med | RTT sum | wall ms |\n|---|---:|---:|---|---:|---:|---:|---:|---:|---:|");
    for (const cat of Object.keys(traced[0].phases.reload.atView)) {
      const c = (f) => cell(traced.map((r) => (r.phases.reload.atView[cat] ? f(r.phases.reload.atView[cat]) : null)));
      console.log(`| ${cat} | ${c((x) => x.n)} | ${c((x) => x.conditional)} | ${JSON.stringify(traced[0].phases.reload.atView[cat].statuses)} | ${c((x) => x.kb)} | ${c((x) => x.queueMs)} | ${c((x) => x.queueSumMs)} | ${c((x) => x.rttMs)} | ${c((x) => x.rttSumMs)} | ${c((x) => x.wallMs)} |`);
    }
  }
}
