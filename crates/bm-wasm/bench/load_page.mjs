// What load_ab.mjs evaluates in the page.
// On each poll: the tiles of the hires manager's view square that the camera's frustum touches (5×5 points on the plane
// at the camera target's height) and when each first had a model.
export const ONSCREEN_RECORDER = `(() => {
  const s = window.__load = { firstVisible: null, allVisible: null, visible: 0, seen: new Map() };
  const visibleSet = (v, m) => {
    const c = v.camera, p = c.projectionMatrix.elements, w = c.matrixWorldInverse.elements;
    const h = v.map.data.hires, y = v.controlsManager.position.y, out = [];
    const on = (x, z) => {
      const vx = w[0] * x + w[4] * y + w[8] * z + w[12], vy = w[1] * x + w[5] * y + w[9] * z + w[13];
      const vz = w[2] * x + w[6] * y + w[10] * z + w[14], vw = w[3] * x + w[7] * y + w[11] * z + w[15];
      const cx = p[0] * vx + p[4] * vy + p[8] * vz + p[12] * vw, cy = p[1] * vx + p[5] * vy + p[9] * vz + p[13] * vw;
      const cw = p[3] * vx + p[7] * vy + p[11] * vz + p[15] * vw;
      return cw > 0 && Math.abs(cx) <= cw && Math.abs(cy) <= cw;
    };
    for (let x = m.centerTile.x - m.viewDistanceX; x <= m.centerTile.x + m.viewDistanceX; x++)
      for (let z = m.centerTile.y - m.viewDistanceZ; z <= m.centerTile.y + m.viewDistanceZ; z++) {
        let hit = false;
        for (let i = 0; i <= 4 && !hit; i++) for (let j = 0; j <= 4 && !hit; j++)
          hit = on((x + i / 4) * h.tileSize.x + h.translate.x, (z + j / 4) * h.tileSize.z + h.translate.z);
        if (hit) out.push(x + "," + z);
      }
    return out;
  };
  const poll = () => {
    try {
      const v = window.bluemap?.mapViewer, m = v?.map?.hiresTileManager;
      if (m && m.tiles.size) {
        const now = performance.now();
        m.tiles.forEach((t) => { const k = t.x + "," + t.z; if (t.model && !s.seen.has(k)) s.seen.set(k, now); });
        const set = visibleSet(v, m);
        s.visible = set.length;
        const missing = set.filter((k) => m.tiles.has("x" + k.replace(",", "z")) ? !s.seen.has(k) : false);
        const known = set.filter((k) => m.tiles.has("x" + k.replace(",", "z")));
        if (s.firstVisible === null && known.some((k) => s.seen.has(k))) s.firstVisible = now;
        if (s.allVisible === null && known.length && !missing.length && m.tiles.size >= set.length * 0.5) s.allVisible = now;
      }
    } catch (e) { s.error = String(e); }
    setTimeout(poll, 20);
  };
  poll();
})()`;

export const onscreenResult = `(() => {
  const s = window.__load;
  const times = [...s.seen.values()].sort((a, b) => a - b);
  return JSON.stringify({ onscreenMs: s.allVisible && Math.round(s.allVisible), firstVisibleMs: s.firstVisible && Math.round(s.firstVisible),
    visible: s.visible, tileP50Ms: times.length ? Math.round(times[times.length >> 1]) : null, onscreenError: s.error });
})()`;

// tiles the hires manager holds without a model once it has gone quiet, and tiles in its spiral's reach it never asked for
export const holes = `(() => {
  const m = window.bluemap.mapViewer.map.hiresTileManager;
  const lows = window.bluemap.mapViewer.map.lowresTileManager;
  const empty = (t) => !t.model && !t.loading;
  const hiresEmpty = [...m.tiles.values()].filter(empty).map((t) => t.x + "," + t.z);
  return JSON.stringify({ hires: m.tiles.size, hiresLoaded: [...m.tiles.values()].filter((t) => t.model).length, hiresEmpty: hiresEmpty.length,
    hiresLoading: m.currentlyLoading, lowres: lows.map((l) => [...l.tiles.values()].filter((t) => t.model).length) });
})()`;
