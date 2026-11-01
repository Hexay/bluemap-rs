  // Fast reload (docs/18). After a browser reload the webapp sends every tile with cache: "no-cache": one round trip
  // per tile over ~6 connections. Here a tile the HTTP cache holds is checked against one batched version list instead;
  // same freshness, every tile is still compared with the server's version. Anything else takes the normal request.
  const TILE_PATH = /^(.*)\/tiles\/(\d+)\/x(-?[\d/]+)z(-?[\d/]+)\.(?:prbm|png)$/;
  // a list is only as fresh as its reply: an entry is good for one use, for this long
  const VERSIONS_TTL_MS = 5000;
  const USED = Symbol("used");
  const batches = new Map(); // tile root path -> live batches

  function managerRect(lod) {
    const map = scope.bluemap?.mapViewer?.map;
    const m = lod === 0 ? map?.hiresTileManager : map?.lowresTileManager?.[lod - 1];
    if (!m || m.unloaded) return null;
    const c = m.centerTile;
    return [lod, c.x - m.viewDistanceX, c.y - m.viewDistanceZ, c.x + m.viewDistanceX, c.y + m.viewDistanceZ];
  }

  function allRects() {
    const lows = scope.bluemap?.mapViewer?.map?.lowresTileManager?.length ?? 0;
    return Array.from({ length: lows + 1 }, (_, lod) => managerRect(lod)).filter(Boolean);
  }

  function startBatch(root, rects, live) {
    const tiles = new Map();
    const batch = { at: performance.now(), rects, tiles, reply: null };
    batch.reply = plainFetch.call(scope, `${root}/tile-versions/${rects.map((r) => r.join(",")).join("/")}`, { cache: "no-store" })
      .then((res) => (res.status === 200 ? res.text() : null))
      .then((text) => {
        if (text === null) return false;
        for (const line of text.split("\n")) {
          const at = line.lastIndexOf(" ");
          if (at > 0) tiles.set(line.slice(0, at), line.slice(at + 1));
        }
        return true;
      })
      .catch(() => false);
    live.push(batch);
    return batch;
  }

  const covers = (rects, lod, x, z) => rects.some((r) => r[0] === lod && x >= r[1] && z >= r[2] && x <= r[3] && z <= r[4]);

  // the tile's version; null: it does not exist; undefined: not known here
  async function versionOf(root, lod, x, z) {
    const now = performance.now();
    const live = (batches.get(root) ?? []).filter((b) => now - b.at < VERSIONS_TTL_MS);
    batches.set(root, live);
    let batch = live.find((b) => covers(b.rects, lod, x, z));
    if (!batch) {
      const rect = managerRect(lod);
      const rects = live.length ? [rect] : allRects();
      if (!rect || !covers(rects, lod, x, z)) return undefined;
      batch = startBatch(root, rects, live);
    }
    if (!(await batch.reply)) return undefined;
    const key = `${lod} ${x} ${z}`;
    const version = batch.tiles.get(key);
    if (version === USED) return undefined;
    batch.tiles.set(key, USED);
    return version === undefined ? null : version;
  }

  // a render changed this tile: what a list says about it is out of date, whoever reads it next
  function tileChanged(lod, x, z) {
    for (const live of batches.values()) for (const b of live) if (covers(b.rects, lod, x, z)) b.tiles.set(`${lod} ${x} ${z}`, USED);
    if (lod !== 0) return;
    heldChanged(x, z);
    // the webapp ignores this event for a tile it is loading; that load is refetched before it is handed over
    if (hiresLoading.has(tileKey(x, z))) hiresDirty.add(tileKey(x, z));
  }

  // the cached reply if the server still has that version, a 204 if it has no such tile; else null
  async function revalidated(request, url) {
    if (request.cache !== "no-cache" || url.origin !== scope.location.origin) return null;
    const m = TILE_PATH.exec(url.pathname);
    if (!m) return null;
    let cached;
    try {
      cached = await plainFetch.call(scope, new Request(request, { cache: "only-if-cached", mode: "same-origin" }));
    } catch {
      return null;
    }
    if (cached.status !== 200 && cached.status !== 204) return null;
    const coord = (s) => Number(s.replaceAll("/", ""));
    const version = await versionOf(m[1], Number(m[2]), coord(m[3]), coord(m[4]));
    if (version === undefined || version === "-") return null;
    if (version === null) return cached.status === 204 ? cached : new Response(null, { status: 204 });
    const etag = cached.headers.get("ETag") ?? "";
    return etag.startsWith(`"${version}"`) || etag.startsWith(`"${version}-`) ? cached : null;
  }
