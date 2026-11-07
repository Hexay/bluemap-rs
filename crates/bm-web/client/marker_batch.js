  // Shape-marker batching, the parts that need no webapp objects (docs/18); markers.js wires them up.
  const shaderIds = new Map();
  const shaderId = (source) => {
    if (!shaderIds.has(source)) shaderIds.set(source, shaderIds.size);
    return shaderIds.get(source);
  };

  // null: a value this cannot compare, so the part keeps drawing itself
  function uniformText(v) {
    if (v === null || v === undefined || typeof v !== "object") return String(v);
    if (v.isColor) return `${v.r},${v.g},${v.b}`;
    if (v.isVector2) return `${v.x},${v.y}`;
    if (v.isVector3) return `${v.x},${v.y},${v.z}`;
    if (v.isVector4 || v.isQuaternion) return `${v.x},${v.y},${v.z},${v.w}`;
    if (v.isMatrix3 || v.isMatrix4) return v.elements.join(",");
    if (Array.isArray(v)) {
      const items = v.map(uniformText);
      return items.includes(null) ? null : `[${items.join(";")}]`;
    }
    return null;
  }

  // the screen size, which the batch sets for all of them each frame
  const PER_FRAME_UNIFORMS = new Set(["resolution"]);

  // Everything of a fill (Mesh) or border (Line2) that changes how it draws, or null if it cannot be merged.
  function partKey(part) {
    const mat = part.material;
    const g = part.geometry;
    if (!mat || Array.isArray(mat) || !mat.uniforms || !g) return null;
    let kind;
    // a Line2 is a Mesh too
    if (part.isLineSegments2) {
      const s = g.attributes?.instanceStart;
      if (!s?.data || s.data.stride !== 6 || s.offset !== 0) return null;
      kind = "border";
    } else if (part.isMesh) {
      const p = g.attributes?.position;
      if (!p || p.isInterleavedBufferAttribute || p.itemSize !== 3 || g.morphAttributes?.position) return null;
      kind = "fill";
    } else {
      return null;
    }
    const out = [
      kind, part.renderOrder, mat.type, shaderId(mat.vertexShader), shaderId(mat.fragmentShader), mat.depthTest,
      mat.depthWrite, mat.transparent, mat.side, mat.blending, mat.opacity, mat.alphaToCoverage, mat.colorWrite,
      mat.polygonOffset, mat.polygonOffsetFactor, mat.polygonOffsetUnits, mat.wireframe, mat.fog, mat.vertexColors,
    ];
    for (const [name, value] of Object.entries(mat.defines ?? {})) out.push(`#${name}=${value}`);
    for (const [name, uniform] of Object.entries(mat.uniforms)) {
      if (PER_FRAME_UNIFORMS.has(name)) continue;
      const text = uniformText(uniform?.value);
      if (text === null) return null;
      out.push(`${name}=${text}`);
    }
    return out.join("|");
  }

  // Of a set's shape markers: what a rebuild depends on (compared with ===), and the parts that can be merged.
  function describeShapes(markers, keyOf = partKey) {
    const signature = [];
    const parts = [];
    for (const marker of markers) {
      const p = marker.position;
      signature.push(marker, marker.visible, p.x, p.y, p.z);
      for (const part of [marker.fill, marker.border]) {
        if (!part) continue;
        const key = marker.visible && part.visible && part.material?.visible !== false ? keyOf(part) : null;
        const q = part.position;
        signature.push(part, part.geometry, part.material, q.x, q.y, q.z, key);
        if (key !== null) parts.push({ marker, part, key });
      }
    }
    return { signature, parts };
  }

  const sameSignature = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);

  function groupBy(parts) {
    const groups = new Map();
    for (const entry of parts) {
      const group = groups.get(entry.key);
      group ? group.push(entry) : groups.set(entry.key, [entry]);
    }
    return groups;
  }

  // point i of `src` through the column-major 4×4 `m` (affine) into point j of `dst`
  function transformPoint(src, i, m, dst, j) {
    const x = src[i], y = src[i + 1], z = src[i + 2];
    dst[j] = m[0] * x + m[4] * y + m[8] * z + m[12];
    dst[j + 1] = m[1] * x + m[5] * y + m[9] * z + m[13];
    dst[j + 2] = m[2] * x + m[6] * y + m[10] * z + m[14];
  }

  // items: { positions (xyz…), index (or null: unindexed), m }; one indexed triangle list in the target space
  function mergeTriangles(items) {
    let vertices = 0;
    let indices = 0;
    for (const it of items) {
      vertices += it.positions.length / 3;
      indices += it.index ? it.index.length : it.positions.length / 3;
    }
    const positions = new Float32Array(vertices * 3);
    const index = new Uint32Array(indices);
    let v = 0;
    let n = 0;
    for (const { positions: src, index: idx, m } of items) {
      const count = src.length / 3;
      for (let k = 0; k < count; k++) transformPoint(src, k * 3, m, positions, (v + k) * 3);
      if (idx) for (let k = 0; k < idx.length; k++) index[n++] = v + idx[k];
      else for (let k = 0; k < count; k++) index[n++] = v + k;
      v += count;
    }
    return { positions, index };
  }

  // items: { positions (start xyz, end xyz per segment), m }; LineSegmentsGeometry.setPositions input
  function mergeSegments(items) {
    const out = new Float32Array(items.reduce((sum, it) => sum + it.positions.length, 0));
    let at = 0;
    for (const { positions: src, m } of items) {
      for (let k = 0; k < src.length; k += 3, at += 3) transformPoint(src, k, m, out, at);
    }
    return out;
  }
