// bluemap-rs client-side unpacking (docs/18). Hires tiles of an optimized storage are stored packed; this asks the
// server for them as stored and unpacks them here, so the server neither unpacks nor recompresses them.
// Loaded ahead of the webapp, whose tile requests go through fetch(). Anything unexpected falls back to plain PRBM.
// A viewer turns it off with: localStorage.setItem("bluemap-rs-client-unpack", "off")
(() => {
  "use strict";
  const MEDIA_TYPE = "application/vnd.bluemap.bmq3";
  const SWITCH = "bluemap-rs-client-unpack";
  const WASM = "__UNPACK_WASM_BASE64__";
  const scope = globalThis;
  const plainFetch = scope.fetch;

  let off = false;
  try {
    off = scope.localStorage.getItem(SWITCH) === "off";
  } catch {
    // storage blocked: keep the default
  }
  if (off || typeof plainFetch !== "function" || typeof WebAssembly !== "object") return;

  let unpacker = null;
  const ready = WebAssembly.instantiate(Uint8Array.from(atob(WASM), (c) => c.charCodeAt(0))).then(
    (module) => {
      unpacker = module.instance.exports;
    },
    () => {},
  );

  // maps/<id>/tiles/0/x…/z….prbm
  const isHiresTile = (url) => /\/tiles\/0\/x[-\d/]+z[-\d/]+\.prbm$/.test(url.pathname);

  const unpack = (body) => {
    const input = unpacker.bmq3_input(body.length);
    new Uint8Array(unpacker.memory.buffer, input, body.length).set(body);
    const length = unpacker.bmq3_unpack();
    if (length < 0) throw new Error("corrupt packed tile");
    // a copy: the module reuses its output buffer
    return new Uint8Array(unpacker.memory.buffer, unpacker.bmq3_output(), length).slice();
  };

  scope.fetch = async function (input, init) {
    let url;
    try {
      url = new URL(typeof input === "string" || input instanceof URL ? input : input.url, scope.location.href);
    } catch {
      return plainFetch.call(this, input, init);
    }
    if (!isHiresTile(url)) return plainFetch.call(this, input, init);
    await ready;
    if (!unpacker) return plainFetch.call(this, input, init);

    const request = new Request(input, init);
    request.headers.set("Accept", `${MEDIA_TYPE}, */*`);
    const response = await plainFetch.call(this, request);
    if (!(response.headers.get("Content-Type") || "").startsWith(MEDIA_TYPE)) return response;
    let prbm;
    try {
      prbm = unpack(new Uint8Array(await response.arrayBuffer()));
    } catch (error) {
      console.warn("bluemap-rs: unpacking a tile failed, loading it unpacked", error);
      return plainFetch.call(this, input, init);
    }
    const headers = new Headers(response.headers);
    headers.set("Content-Type", "application/octet-stream");
    headers.set("Content-Length", String(prbm.length));
    headers.delete("Content-Encoding");
    return new Response(prbm, { status: response.status, statusText: response.statusText, headers });
  };
})();
