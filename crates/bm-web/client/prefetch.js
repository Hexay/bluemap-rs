  // Fetch-ahead (docs/18): while the camera moves, fetches tiles the webapp is about to ask for, in the same form, and
  // answers it from here. The camera and manager are read from window.bluemap; without them nothing runs.
  const AHEAD_SECONDS = 1;
  const AHEAD_MIN_SPEED = 8; // blocks/s
  const AHEAD_MAX_IN_FLIGHT = 3;
  const HELD_MAX = 16; // unclaimed tiles, in flight or done: the bound on waste at any moment
  const HELD_TTL_MS = 15000;
  const NO_CACHE_FRESH_MS = 3000; // a "no-cache" request accepts a held tile this young, fetched no-cache itself
  const WASTE_CHECK_AFTER = 24;
  const WASTE_LIMIT = 0.35;
  const WASTE_PAUSE_MS = 10000;
  const stats = { issued: 0, claimed: 0, wasted: 0, aborted: 0 };
  scope.__bluemapRsPrefetch = stats;
  const held = new Map(); // "x z" -> { promise, controller, at, noCache, done, ahead }
  let tileRoot = null;

  const tileKey = (x, z) => `${x} ${z}`;
  const pathPart = (n) => (n < 0 ? "-" : "") + [...String(Math.abs(n))].join("/") + "/";
  const tileUrl = (x, z) => `${tileRoot}/tiles/0/${`x${pathPart(x)}z${pathPart(z)}`.slice(0, -1)}.prbm`;

  function hiresView() {
    const viewer = scope.bluemap?.mapViewer;
    const manager = viewer?.map?.hiresTileManager;
    if (!manager || manager.unloaded || manager.viewDistanceX <= 0 || !viewer.controlsManager) return null;
    return { viewer, manager };
  }

  // the order the manager walks its square; it never reaches some corners, so those are never prefetched
  function spiral(dx, dz) {
    const out = [];
    let x = 0, z = 0, d = 1, m = 1;
    const push = () => Math.abs(x) <= dx && Math.abs(z) <= dz && out.push([x, z]);
    while (m < Math.max(dx, dz) * 2 + 1) {
      while (2 * x * d < m) (push(), (x += d));
      while (2 * z * d < m) (push(), (z += d));
      d = -d;
      m++;
    }
    return out;
  }

  function fetchHeld(x, z, noCache) {
    const controller = new AbortController();
    const headers = { Accept: `${FRAME_TYPE}, ${MEDIA_TYPE}, */*` };
    const entry = { controller, at: performance.now(), noCache, done: false };
    stats.issued++;
    entry.promise = plainFetch.call(scope, tileUrl(x, z), { headers, signal: controller.signal, priority: "low",
      cache: noCache ? "no-cache" : "default", credentials: "same-origin" })
      .then(async (res) => {
        if (res.status !== 200 && res.status !== 204) return null;
        const body = res.status === 200 ? await res.arrayBuffer() : null;
        return new Response(body, { status: res.status, statusText: res.statusText, headers: res.headers });
      })
      .catch(() => null)
      .finally(() => {
        entry.done = true;
        pump();
      });
    held.set(tileKey(x, z), entry);
  }

  function dropHeld(key, entry, count) {
    held.delete(key);
    if (!entry.done) entry.controller.abort();
    stats[count]++;
  }

  // the webapp's own request takes over; null: nothing usable held
  function claimHeld(x, z, url, request) {
    const key = tileKey(x, z);
    const entry = held.get(key);
    if (!entry) return null;
    held.delete(key);
    if (`${tileRoot}/` !== url.pathname.slice(0, tileRoot.length + 1)) return null;
    if (request.cache === "no-cache" && !(entry.noCache && performance.now() - entry.at < NO_CACHE_FRESH_MS)) return null;
    stats.claimed++;
    return entry.promise;
  }

  function heldChanged(x, z) {
    const entry = held.get(tileKey(x, z));
    if (entry) dropHeld(tileKey(x, z), entry, "aborted");
  }

  let queue = []; // [x, z], next first
  function pump() {
    const view = hiresView();
    if (!view || tileRoot === null) return;
    const flying = () => [...held.values()].filter((e) => !e.done).length;
    while (queue.length && held.size < HELD_MAX && flying() < AHEAD_MAX_IN_FLIGHT) {
      const [x, z] = queue.shift();
      if (!held.has(tileKey(x, z)) && !view.manager.tiles.has(`x${x}z${z}`)) fetchHeld(x, z, !!view.viewer.revalidatedUrls);
    }
  }

  let last = null;
  let dirX = 0, dirZ = 0;
  let aimed = "";
  let pausedUntil = 0;

  function steer(view, now) {
    const pos = view.viewer.controlsManager.position;
    if (last) {
      const dt = (now - last.t) / 1e3;
      const gain = Math.min(1, dt / 0.3);
      dirX += ((pos.x - last.x) / dt - dirX) * gain;
      dirZ += ((pos.z - last.z) / dt - dirZ) * gain;
    }
    last = { x: pos.x, z: pos.z, t: now };
    const speed = Math.hypot(dirX, dirZ);
    if (speed < AHEAD_MIN_SPEED || now < pausedUntil || scope.bluemap.appState?.controls?.pauseTileLoading || scope.navigator?.connection?.saveData) {
      queue = [];
      aimed = "";
      return;
    }
    for (const [key, entry] of held) {
      if (entry.done) continue;
      const [x, z] = key.split(" ").map(Number);
      const h = view.viewer.map.data.hires;
      const off = [(x + 0.5) * h.tileSize.x - pos.x, (z + 0.5) * h.tileSize.z - pos.z];
      if (off[0] * dirX + off[1] * dirZ < 0 && Math.hypot(...off) > 64) dropHeld(key, entry, "aborted"); // the view turned away from it
    }
    const h = view.viewer.map.data.hires, m = view.manager;
    const tx = Math.floor((pos.x + dirX * AHEAD_SECONDS - h.translate.x) / h.tileSize.x);
    const tz = Math.floor((pos.z + dirZ * AHEAD_SECONDS - h.translate.z) / h.tileSize.z);
    const aim = `${tx} ${tz}`;
    if (aim === aimed) return;
    aimed = aim;
    queue = spiral(m.viewDistanceX, m.viewDistanceZ).map(([a, b]) => [a + tx, b + tz])
      .filter(([a, b]) => !m.tiles.has(`x${a}z${b}`) && (a - m.centerTile.x) * dirX + (b - m.centerTile.y) * dirZ > 0);
  }

  function tick() {
    const now = performance.now();
    for (const [key, entry] of held) if (now - entry.at > HELD_TTL_MS) dropHeld(key, entry, "wasted");
    if (stats.issued >= WASTE_CHECK_AFTER && stats.wasted + stats.aborted > WASTE_LIMIT * stats.issued && now >= pausedUntil) {
      pausedUntil = now + WASTE_PAUSE_MS;
      Object.assign(stats, { issued: 0, claimed: 0, wasted: 0, aborted: 0, pauses: (stats.pauses ?? 0) + 1 });
    }
    const view = hiresView();
    if (!view) return;
    steer(view, now);
    pump();
  }

  // every hires request of the webapp lands here: it names the tile root
  function noteHiresRequest(url, pathRoot, x, z) {
    if (tileRoot === pathRoot) return;
    for (const [key, entry] of held) dropHeld(key, entry, "aborted");
    queue = [];
    // the path builder has to match the webapp's, or nothing here is safe to ask for
    tileRoot = pathRoot;
    if (tileUrl(x, z) !== url.pathname) tileRoot = null;
  }

  if (TUNING.ahead) {
    setInterval(() => {
      try {
        tick();
      } catch {
        queue = []; // a webapp that doesn't look as expected just gets no prefetch
      }
    }, 100);
  }
