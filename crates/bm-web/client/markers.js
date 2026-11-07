  // Shape-marker batching (docs/18, BlueMap#904): each ShapeMarker draws a fill and a border with materials of its
  // own, ~18.6k draw calls for 9.3k claims. A set with at least TUNING.markerBatch of them gets one merged fill and one
  // merged border per look, children of the set so its toggle still applies; the originals move to a layer the camera
  // does not render but the webapp's raycaster (layers.enableAll) still hits, so clicks and popups are upstream's.
  const HIDDEN_LAYER = 30;
  const markerBatches = new WeakMap();
  const appliedData = new WeakMap();
  let three = null;

  // three.js classes out of the webapp's bundle, through the ShapeMarker parts (ShapeMarkerFill extends Mesh,
  // ShapeMarkerBorder extends Line2)
  function threeOf(marker) {
    const { fill, border } = marker;
    const t = {
      Mesh: Object.getPrototypeOf(fill.constructor),
      BufferGeometry: Object.getPrototypeOf(fill.geometry.constructor),
      BufferAttribute: Object.getPrototypeOf(fill.geometry.attributes.position.constructor),
      Line2: Object.getPrototypeOf(border.constructor),
      LineSegmentsGeometry: border.geometry.constructor,
      Matrix4: fill.matrixWorld.constructor,
    };
    const ok = new t.Mesh().isMesh && new t.Line2().isLineSegments2 && new t.BufferGeometry().isBufferGeometry;
    return ok ? t : null;
  }

  const setFrozen = (object, frozen) => {
    object.matrixAutoUpdate = !frozen;
    object.matrixWorldAutoUpdate = !frozen;
  };

  function unbatch(set, entry) {
    for (const object of entry.objects) {
      set.remove(object);
      object.dispose();
    }
    for (const { part } of entry.parts) part.layers.set(0);
    for (const marker of entry.frozen) [marker, marker.fill, marker.border].forEach((o) => o && setFrozen(o, false));
  }

  // a batch draws, the originals answer clicks
  function batchObject(object, geometry, material, renderOrder) {
    object.renderOrder = renderOrder;
    object.matrixAutoUpdate = false;
    object.raycast = () => {};
    object.dispose = () => {
      geometry.dispose();
      material.dispose();
    };
    return object;
  }

  function buildGroup(t, group, toSet) {
    const first = group[0].part;
    const rel = new t.Matrix4();
    const material = first.material.clone();
    const items = group.map(({ part }) => {
      const m = rel.multiplyMatrices(toSet, part.matrixWorld).elements.slice();
      const g = part.geometry;
      if (part.isLineSegments2) return { positions: g.attributes.instanceStart.data.array, m };
      return { positions: g.attributes.position.array, index: g.index?.array ?? null, m };
    });
    if (!first.isLineSegments2) {
      const { positions, index } = mergeTriangles(items);
      const geometry = new t.BufferGeometry();
      geometry.setAttribute("position", new t.BufferAttribute(positions, 3));
      geometry.setIndex(new t.BufferAttribute(index, 1));
      geometry.computeBoundingSphere();
      return batchObject(new t.Mesh(geometry, material), geometry, material, first.renderOrder);
    }
    const geometry = new t.LineSegmentsGeometry();
    geometry.setPositions(mergeSegments(items));
    const line = batchObject(new t.Line2(geometry, material), geometry, material, first.renderOrder);
    if (material.dashed) line.computeLineDistances();
    // line widths are screen pixels (ShapeMarkerBorder.onBeforeRender); the hidden originals get the size for raycasts
    let width = -1;
    let height = -1;
    line.onBeforeRender = (renderer) => {
      const r = renderer.getSize(material.resolution);
      if (r.x === width && r.y === height) return;
      [width, height] = [r.x, r.y];
      group.forEach(({ part }) => part.material.resolution?.set(width, height));
    };
    return line;
  }

  function build(set, markers, parts, signature) {
    set.updateWorldMatrix(true, false);
    for (const marker of markers) {
      [marker, marker.fill, marker.border].forEach((o) => o && setFrozen(o, false));
      marker.updateMatrixWorld(true);
    }
    const toSet = new three.Matrix4().copy(set.matrixWorld).invert();
    // all built before anything changes: a throw leaves the set as upstream draws it
    const objects = [...groupBy(parts).values()].map((group) => buildGroup(three, group, toSet));
    objects.forEach((object) => set.add(object));
    for (const { part } of parts) part.layers.set(HIDDEN_LAYER);
    // matrices current as of now; any move changes the signature, and the rebuild recomputes them
    const frozen = [...new Set(parts.map((p) => p.marker))];
    for (const marker of frozen) [marker, marker.fill, marker.border].forEach((o) => o && setFrozen(o, true));
    return { signature, objects, parts, frozen };
  }

  function batchSet(set) {
    const markers = [];
    set.markers?.forEach((m) => m.isShapeMarker && markers.push(m));
    const old = markerBatches.get(set);
    if (!(TUNING.markerBatch > 0 && markers.length >= TUNING.markerBatch)) {
      if (old) unbatch(set, old);
      markerBatches.delete(set);
      return;
    }
    three ??= threeOf(markers[0]);
    if (!three) return;
    const { signature, parts } = describeShapes(markers);
    if (old && sameSignature(old.signature, signature)) return;
    if (old) unbatch(set, old);
    markerBatches.delete(set);
    markerBatches.set(set, build(set, markers, parts, signature));
  }

  // must not throw: the webapp's MarkerManager.update clears all markers on an error
  function rebatch(set) {
    try {
      if (!set?.isMarkerSet) return;
      batchSet(set);
      set.markerSets?.forEach(rebatch);
    } catch (error) {
      console.warn("bluemap-rs: batching shape markers failed", error);
    }
  }

  // NormalMarkerManager.updateFromData, for the file poll and SSE "marker" events; a map switch makes a new manager
  // of the same class, so its prototype is patched once
  function hookMarkerManager(manager) {
    const proto = Object.getPrototypeOf(manager);
    const update = proto.updateFromData;
    if (update.bmrs) return;
    const wrapped = function (data) {
      // marker_poll.js hands an unchanged file over as the very object applied last
      if (TUNING.markerPoll && data !== null && typeof data === "object" && appliedData.get(this) === data) return true;
      appliedData.set(this, data);
      const result = update.call(this, data);
      rebatch(this.root);
      return result;
    };
    wrapped.bmrs = true;
    proto.updateFromData = wrapped;
    rebatch(manager.root);
  }

  if (TUNING.markerBatch > 0 || TUNING.markerPoll) {
    setInterval(() => {
      const manager = scope.bluemap?.markerFileManager;
      if (typeof manager?.updateFromData === "function") hookMarkerManager(manager);
    }, 100);
  }
