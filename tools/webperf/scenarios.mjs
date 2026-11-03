// Camera scenarios, driven in the page through the webapp's ControlsManager (position/distance/angle/rotation) once
// per animation frame, deterministic in time. Coordinates are relative to the map's startPos; `extent` is the radius
// in blocks. Each returns `{ seconds, path }` where path is the source of `(t, c, R) => camera`.
export const SCENARIOS = {
  // fast top-ish pan around a square of side 2R at 100 blocks/s, hires distance
  pan: (o) => ({
    seconds: o.seconds ?? Math.min(60, (8 * o.extent) / 100),
    path: `(t, c, R) => {
      const L = 8 * R, d = (t * 100) % L, side = Math.floor(d / (2 * R)), u = d % (2 * R) - R;
      const [x, z] = [[u, -R], [R, u], [-u, R], [-R, -u]][side];
      return { x: c.x + x, z: c.z + z, distance: 300, angle: 0.6, rotation: 0 };
    }`,
  }),
  // zoom from 100 out to 10000 (top lowres LOD) and back, exponential, slowly orbiting
  zoom: (o) => ({
    seconds: o.seconds ?? 20,
    path: `(t, c, R, T) => {
      const u = t / T, k = u < 0.5 ? u * 2 : 2 - u * 2;
      return { x: c.x, z: c.z, distance: 100 * Math.pow(100, k), angle: 0.5, rotation: t * 0.1 };
    }`,
  }),
  // low, near-horizontal flight along a diagonal at 40 blocks/s, looking ahead
  flyover: (o) => ({
    seconds: o.seconds ?? 20,
    path: `(t, c, R, T) => {
      const u = -R + (2 * R * t) / T;
      return { x: c.x + u * 0.7, z: c.z + u * 0.7, distance: 40, angle: 1.35, rotation: Math.PI * 0.75 };
    }`,
  }),
  // no movement, no redraw requests: what the page costs while someone just looks at it
  idle: (o) => ({ seconds: o.seconds ?? 10, path: null }),
  // seeded random roam between waypoints at 150 blocks/s, distance 100-3000
  soak: (o) => ({
    seconds: o.seconds ?? (o.minutes ?? 5) * 60,
    path: `(() => {
      let seed = 12345;
      const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
      const legs = [];
      return (t, c, R) => {
        while (!legs.length || legs[legs.length - 1].end <= t) {
          const a = legs.length ? legs[legs.length - 1].to : { x: c.x, z: c.z, d: 300 };
          const b = { x: c.x + (rnd() * 2 - 1) * R, z: c.z + (rnd() * 2 - 1) * R, d: 100 * Math.pow(30, rnd()) };
          const start = legs.length ? legs[legs.length - 1].end : 0;
          legs.push({ from: a, to: b, start, end: start + Math.hypot(b.x - a.x, b.z - a.z) / 150 + 2 });
          if (legs.length > 2) legs.shift();
        }
        const l = legs.find((l) => t < l.end);
        const u = Math.min(1, (t - l.start) / (l.end - l.start - 2));
        return { x: l.from.x + (l.to.x - l.from.x) * u, z: l.from.z + (l.to.z - l.from.z) * u,
          distance: l.from.d + (l.to.d - l.from.d) * u, angle: 0.6, rotation: 0 };
      };
    })()`,
  }),
};

// In-page: runs a scenario, resolves when it ends.
export const driveExpression = (path, seconds, extent) => `new Promise((resolve) => {
  const v = window.bluemap.mapViewer, cm = v.controlsManager, sp = v.map.data.startPos;
  const c = { x: sp.x, z: sp.z };
  const path = ${path ?? "null"};
  const T = ${seconds}, R = ${extent}, t0 = performance.now();
  const step = (now) => {
    const t = (now - t0) / 1000;
    if (t >= T) return resolve("done");
    if (path) {
      const p = path(t, c, R, T);
      cm.position.x = p.x;
      cm.position.z = p.z;
      cm.distance = p.distance;
      cm.angle = p.angle;
      cm.rotation = p.rotation;
      v.redraw();
    }
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
})`;
