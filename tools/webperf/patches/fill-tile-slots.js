// Candidate route-A fix (docs/25): the webapp's TileManager.loadCloseTiles starts ONE tile per call and re-arms itself
// with setTimeout(0), so starting N tiles takes N timer tasks; when frames are long the scheduler runs about one
// such task per frame, and tile starts are tied to the frame rate. This fills every free slot (8 per manager) per call.
(() => {
  const patched = new WeakSet();
  const patch = (m) => {
    if (!m || patched.has(m)) return;
    patched.add(m);
    m.loadCloseTiles = () => {
      if (m.unloaded) return;
      if (m.loadTimeout) clearTimeout(m.loadTimeout);
      m.loadTimeout = null;
      while (m.currentlyLoading < 8 && m.loadNextTile());
      // as upstream: with every slot busy, look again in 1 s (a finished tile also calls this, via setTimeout 0)
      if (m.currentlyLoading >= 8) m.loadTimeout = setTimeout(m.loadCloseTiles, 1000);
    };
  };
  setInterval(() => {
    const map = window.bluemap?.mapViewer?.map;
    if (!map?.hiresTileManager) return;
    patch(map.hiresTileManager);
    (map.lowresTileManager ?? []).forEach(patch);
  }, 50);
})();
