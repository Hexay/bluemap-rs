  // Tile slots (docs/25): the webapp's TileManager.loadCloseTiles starts one tile, then re-arms itself with
  // setTimeout(0); with slow frames about one such timer runs per frame, so tile loading is paced by the frame rate
  // (first view 13.8 s → 4.7 s in software GL). This version fills every free slot per call, same limit of 8.
  const slotted = new WeakSet();

  function fillSlots(m) {
    if (!m || slotted.has(m) || typeof m.loadNextTile !== "function") return;
    slotted.add(m);
    m.loadCloseTiles = () => {
      if (m.unloaded) return;
      if (m.loadTimeout) clearTimeout(m.loadTimeout);
      m.loadTimeout = null;
      while (m.currentlyLoading < 8 && m.loadNextTile());
      if (m.currentlyLoading >= 8) m.loadTimeout = setTimeout(m.loadCloseTiles, 1000);
    };
  }

  // a map switch brings new tile managers
  if (TUNING.slots) {
    setInterval(() => {
      const map = scope.bluemap?.mapViewer?.map;
      if (!map?.hiresTileManager) return;
      fillSlots(map.hiresTileManager);
      (map.lowresTileManager ?? []).forEach(fillSlots);
    }, 100);
  }
