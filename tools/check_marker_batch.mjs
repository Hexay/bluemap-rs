// usage: node tools/check_marker_batch.mjs [three package dir]
// The client script's shape-marker batching and markers.json poll skip (docs/18) against real three.js (r186, the
// webapp's) and stand-ins for the webapp's ShapeMarker/MarkerSet/NormalMarkerManager built like upstream's. No browser:
// checks grouping, merged vertices, hidden-but-clickable originals, rebuild/skip/unbatch, and the fetch wrapper.
// Also compiles the assembled script. Exits 1 on any failure.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import vm from "node:vm";

const client = join(import.meta.dirname, "../crates/bm-web/client");
const threeDir = process.argv[2] ?? "/srv/agents/work/BlueMap-upstream/common/webapp/node_modules/three";
const THREE = await import(pathToFileURL(join(threeDir, "build/three.module.js")).href);
const { Line2 } = await import(pathToFileURL(join(threeDir, "examples/jsm/lines/Line2.js")).href);
const { LineMaterial } = await import(pathToFileURL(join(threeDir, "examples/jsm/lines/LineMaterial.js")).href);
const { LineSegmentsGeometry } = await import(pathToFileURL(join(threeDir, "examples/jsm/lines/LineSegmentsGeometry.js")).href);
const { Color, DoubleSide, Mesh, Object3D, PerspectiveCamera, Raycaster, Scene, Shape, ShapeGeometry, ShaderMaterial, Vector2, Vector3 } = THREE;

// PARTS order of crates/bm-web/src/client_unpack.rs
const rs = readFileSync(join(client, "../src/client_unpack.rs"), "utf8");
const partFiles = [...rs.matchAll(/include_str!\("\.\.\/client\/([\w.]+\.js)"\)/g)].map((m) => m[1]).filter((f) => f !== "unpack.js");
const read = (f) => readFileSync(join(client, f), "utf8");
new vm.Script(read("unpack.js").replace("// __CLIENT_PARTS__", partFiles.map(read).join("")), { filename: "assembled.js" });
assert.deepEqual(partFiles.slice(-3), ["marker_batch.js", "markers.js", "marker_poll.js"]);

// the three parts in one scope, as in the script
const TUNING = { markerBatch: 64, markerPoll: 1 };
const fetched = [];
let answer = null;
const plainFetch = async (request) => (fetched.push(request), answer(request));
const scope = { location: { href: "http://h/", origin: "http://h" } };
const internals = new Function("scope", "TUNING", "plainFetch", "setInterval",
  ["marker_batch.js", "markers.js", "marker_poll.js"].map(read).join("") +
  "return { markerBatches, hookMarkerManager, fetchLiveMarkers, HIDDEN_LAYER };")(scope, TUNING, plainFetch, () => {});
const { markerBatches, hookMarkerManager, fetchLiveMarkers, HIDDEN_LAYER } = internals;

// --- stand-ins, shaped like markers/ShapeMarker.js, MarkerSet.js and NormalMarkerManager.js
class Fill extends Mesh {
  constructor() {
    super(new ShapeGeometry(new Shape([new Vector2(), new Vector2(1, 0), new Vector2(0, 1)])), new ShaderMaterial({
      vertexShader: "void main(){}", fragmentShader: "void main(){}", side: DoubleSide, transparent: true, depthTest: true,
      uniforms: { markerColor: { value: new Color() }, markerOpacity: { value: 0 }, fadeDistanceMin: { value: 0 },
        fadeDistanceMax: { value: Number.MAX_VALUE } },
    }));
  }
  dispose() { this.geometry.dispose(); this.material.dispose(); }
}
class Border extends Line2 {
  constructor() {
    const m = new LineMaterial({ color: new Color(), opacity: 0, transparent: true, linewidth: 1, dashed: false });
    m.uniforms.fadeDistanceMin = { value: 0 };
    m.uniforms.fadeDistanceMax = { value: Number.MAX_VALUE };
    m.resolution.set(1024, 768);
    super(new LineSegmentsGeometry(), m);
  }
  onBeforeRender(renderer) { renderer.getSize(this.material.resolution); }
  dispose() { this.geometry.dispose(); this.material.dispose(); }
}
class ShapeMarker extends Object3D {
  constructor() {
    super();
    this.isShapeMarker = true;
    this.fill = new Fill();
    this.border = new Border();
    this.border.renderOrder = -1;
    this.add(this.border, this.fill);
  }
  updateFromData(d) {
    this.position.set(d.position.x, d.position.y, d.position.z);
    const key = JSON.stringify([d.shape, d.position]);
    if (key !== this.shapeKey) {
      this.shapeKey = key;
      const shape = new Shape(d.shape.map((p) => new Vector2(p.x - d.position.x, p.z - d.position.z)));
      this.fill.geometry.dispose();
      this.fill.geometry = new ShapeGeometry(shape, 5).rotateX(Math.PI / 2);
      const pts = shape.getPoints(5);
      pts.push(pts[0]);
      const segs = pts.slice(1).flatMap((p, i) => [pts[i].x, 0, pts[i].y, p.x, 0, p.y]);
      this.border.geometry = new LineSegmentsGeometry().setPositions(segs);
    }
    this.fill.position.y = this.border.position.y = d.shapeY - d.position.y + 0.01;
    const lc = d.lineColor, fc = d.fillColor;
    this.border.material.color.setRGB(lc.r / 255, lc.g / 255, lc.b / 255);
    this.border.material.opacity = lc.a;
    this.border.visible = lc.a > 0;
    this.border.material.linewidth = d.lineWidth;
    this.fill.material.uniforms.markerColor.value.setRGB(fc.r / 255, fc.g / 255, fc.b / 255);
    this.fill.material.uniforms.markerOpacity.value = fc.a;
    this.fill.visible = fc.a > 0;
  }
  dispose() { this.fill.dispose(); this.border.dispose(); }
}
class MarkerSet extends Scene {
  constructor() { super(); this.isMarkerSet = true; this.markers = new Map(); this.markerSets = new Map(); }
  updateFromData(data) {
    for (const [id, d] of Object.entries(data.markers)) {
      let m = this.markers.get(id);
      if (!m) this.markers.set(id, (m = new ShapeMarker())), this.add(m);
      m.updateFromData(d);
    }
    for (const [id, m] of this.markers) if (!(id in data.markers)) this.markers.delete(id), this.remove(m), m.dispose();
  }
  dispose() { this.children.forEach((c) => c.dispose?.()); }
}
class Manager {
  constructor(root) { this.root = root; this.applied = 0; }
  updateFromData(data) {
    this.applied++;
    for (const [id, d] of Object.entries(data)) {
      let set = this.root.markerSets.get(id);
      if (!set) this.root.markerSets.set(id, (set = new MarkerSet())), this.root.add(set);
      set.updateFromData(d);
    }
    for (const [id, s] of this.root.markerSets) if (!(id in data)) this.root.markerSets.delete(id), this.root.remove(s), s.dispose();
    return true;
  }
}

const STYLES = [
  { lineColor: { r: 255, g: 0, b: 0, a: 1 }, fillColor: { r: 200, g: 0, b: 0, a: 0.3 }, lineWidth: 2 },
  { lineColor: { r: 0, g: 0, b: 255, a: 1 }, fillColor: { r: 0, g: 0, b: 200, a: 0.3 }, lineWidth: 2 },
  { lineColor: { r: 0, g: 0, b: 255, a: 1 }, fillColor: { r: 0, g: 0, b: 200, a: 0.3 }, lineWidth: 3 },
];
function claims(count, edit = () => {}) {
  const markers = {};
  for (let i = 0; i < count; i++) {
    const x = (i % 30) * 40, z = Math.floor(i / 30) * 40, w = 8 + (i % 5) * 4;
    const d = { type: "shape", position: { x: x + w / 2, y: 64, z: z + w / 2 }, shapeY: 64, ...structuredClone(STYLES[i % 3]),
      shape: [{ x, z }, { x: x + w, z }, { x: x + w, z: z + w }, { x, z: z + w }] };
    edit(d, i);
    markers[`claim-${i}`] = d;
  }
  return { claims: { label: "Claims", markers }, few: { label: "Few", markers: Object.fromEntries(Object.entries(markers).slice(0, 5)) } };
}

const root = new MarkerSet();
const manager = new Manager(root);
manager.updateFromData(claims(100));
hookMarkerManager(manager);
const set = root.markerSets.get("claims");
const few = root.markerSets.get("few");
const batchObjects = (s) => s.children.filter((c) => !c.isShapeMarker);
const isFill = (o) => o.isMesh && !o.isLineSegments2;
const markersOf = (s) => [...s.markers.values()];

// 3 looks (style 1 and 2 share a fill, not a border) → 2 fills + 3 borders; small set untouched
let objects = batchObjects(set);
assert.equal(objects.filter((o) => isFill(o)).length, 2);
assert.equal(objects.filter((o) => o.isLineSegments2).length, 3);
assert.equal(batchObjects(few).length, 0);
assert.ok(markersOf(few).every((m) => m.fill.layers.mask === 1 && m.matrixAutoUpdate));

// merged fill = every original vertex in set space
const red = objects.find((o) => isFill(o) &&o.material.uniforms.markerColor.value.r > 0.5);
const reds = markersOf(set).filter((_, i) => i % 3 === 0);
assert.equal(red.geometry.attributes.position.count, reds.reduce((n, m) => n + m.fill.geometry.attributes.position.count, 0));
const v0 = new Vector3().fromBufferAttribute(reds[1].fill.geometry.attributes.position, 0).applyMatrix4(reds[1].fill.matrixWorld);
const offset = reds[0].fill.geometry.attributes.position.count;
assert.ok(new Vector3().fromBufferAttribute(red.geometry.attributes.position, offset).distanceTo(v0) < 1e-4);
assert.equal(red.geometry.index.count, reds.reduce((n, m) => n + m.fill.geometry.index.count, 0));
assert.ok(Math.abs(v0.y - 64.01) < 1e-4, "shapeY offset kept");
const border = objects.find((o) => o.isLineSegments2 && o.material.linewidth === 3);
assert.equal(border.geometry.instanceCount, markersOf(set).filter((_, i) => i % 3 === 2).length * 4);
assert.equal(border.renderOrder, -1);

// originals: not drawn by the camera, hit by the webapp's raycaster; the batch is never hit
const camera = new PerspectiveCamera();
const target = markersOf(set)[7];
assert.ok(!target.fill.layers.test(camera.layers) && !target.border.layers.test(camera.layers));
assert.equal(target.fill.layers.mask, 1 << HIDDEN_LAYER);
assert.ok(objects.every((o) => o.layers.test(camera.layers)));
const raycaster = new Raycaster(new Vector3(target.position.x + 0.5, 200, target.position.z + 0.5), new Vector3(0, -1, 0));
raycaster.layers.enableAll();
raycaster.camera = camera;
const hits = raycaster.intersectObjects([root], true).map((h) => h.object);
assert.ok(hits.includes(target.fill), "hidden fill still clickable");
assert.ok(!hits.some((o) => objects.includes(o)));
assert.ok(!target.matrixAutoUpdate && !target.fill.matrixWorldAutoUpdate, "hidden originals frozen");

// screen size: batch sets it, the originals follow for their raycasts
border.onBeforeRender({ getSize: (v) => v.set(800, 600) });
assert.equal(border.material.resolution.x, 800);
assert.ok(markersOf(set).filter((_, i) => i % 3 === 2).every((m) => m.border.material.resolution.y === 600));

// same object again: skipped; same content parsed anew: applied, no rebuild
const data = claims(100);
manager.updateFromData(data);
const applied = manager.applied;
manager.updateFromData(data);
assert.equal(manager.applied, applied);
assert.deepEqual(batchObjects(set), objects);
manager.updateFromData(claims(100));
assert.equal(manager.applied, applied + 1);
assert.deepEqual(batchObjects(set), objects, "unchanged data keeps the batch");

// a colour change and a move rebuild it and dispose the old batch
let disposed = 0;
objects.forEach((o) => o.geometry.addEventListener("dispose", () => disposed++));
manager.updateFromData(claims(100, (d, i) => {
  if (i === 0) d.fillColor = { r: 0, g: 255, b: 0, a: 0.5 };
  if (i === 7) (d.position.x += 1000), d.shape.forEach((p) => (p.x += 1000));
}));
assert.equal(disposed, 5);
objects = batchObjects(set);
assert.equal(objects.filter((o) => isFill(o)).length, 3);
assert.equal(target.matrixWorld.elements[12], target.position.x, "moved marker's matrix recomputed");
assert.ok(target.position.x > 1000);

// invisible parts and hidden markers stay out
markersOf(set)[3].visible = false;
manager.updateFromData(claims(100, (d, i) => i === 4 && (d.lineColor.a = 0)));
const drawn = batchObjects(set).filter((o) => o.isLineSegments2).reduce((n, o) => n + o.geometry.instanceCount, 0);
assert.equal(drawn, (100 - 2) * 4);
assert.equal(markersOf(set)[3].fill.layers.mask, 1, "a hidden marker's parts are left as they are");

// below the threshold: back to upstream's objects
manager.updateFromData(claims(40));
assert.equal(batchObjects(root.markerSets.get("claims")).length, 0);
assert.ok(markersOf(root.markerSets.get("claims")).every((m) => m.fill.layers.mask === 1 && m.matrixAutoUpdate && m.fill.matrixWorldAutoUpdate));
assert.ok(!markerBatches.has(root.markerSets.get("claims")));

// set removed: its batch is disposed with it
manager.updateFromData(claims(100));
disposed = 0;
batchObjects(set).forEach((o) => o.geometry.addEventListener("dispose", () => disposed++));
manager.updateFromData({});
assert.equal(disposed, 5);

// --- markers.json fetches
const url = "http://h/maps/w/live/markers.json";
const reply = (body, status = 200, etag = null) => () =>
  new Response(status === 304 ? null : body, { status, headers: etag ? { ETag: etag } : {} });
answer = reply('{"a":{}}', 200, '"t1"');
const first = await fetchLiveMarkers(null, new Request(url, { cache: "no-cache" }));
assert.equal(first.body, undefined, "no stream: the loader calls json() on it");
const a = await first.json();
assert.deepEqual(a, { a: {} });
answer = reply(null, 304, '"t1"');
const second = await fetchLiveMarkers(null, url);
assert.equal(fetched.at(-1).headers.get("If-None-Match"), '"t1"');
assert.equal(second.status, 200);
assert.equal(await second.json(), a, "304: the object parsed last");
answer = reply('{"b":{}}', 200, '"t2"');
assert.deepEqual(await (await fetchLiveMarkers(null, url)).json(), { b: {} });
// a server without ETags: body compared
const java = "http://h/maps/j/live/markers.json";
answer = reply('{"c":{}}');
const c = await (await fetchLiveMarkers(null, java)).json();
assert.equal(fetched.at(-1).headers.get("If-None-Match"), null);
assert.equal(await (await fetchLiveMarkers(null, java)).json(), c);
answer = reply('{"c":{"x":1}}');
assert.notEqual(await (await fetchLiveMarkers(null, java)).json(), c);
answer = reply("", 404);
assert.equal((await fetchLiveMarkers(null, java)).status, 404);
answer = reply("{broken");
await assert.rejects((await fetchLiveMarkers(null, java)).json());

console.log("marker batching and markers.json polls: ok");
