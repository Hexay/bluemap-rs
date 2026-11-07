  // markers.json polls (docs/18): the webapp fetches live/markers.json every 10 s (served no-store) and re-parses and
  // re-applies all of it. An unchanged file (304 to our ETag, or the same text from a server without one) is handed
  // over as the object parsed last, which the wrapped updateFromData (markers.js) skips.
  const liveMarkers = new Map();
  const isLiveMarkers = (url) => /\/live\/markers\.json$/.test(url.pathname);

  // what RevalidatingFileLoader reads: no body stream, so it calls json() on this very response
  class CachedJson extends Response {
    #entry;
    constructor(entry, init) {
      super(null, init);
      this.#entry = entry;
    }
    get body() {
      return undefined;
    }
    json() {
      return Promise.resolve(this.#entry.json);
    }
    text() {
      return Promise.resolve(this.#entry.text ?? JSON.stringify(this.#entry.json));
    }
    clone() {
      return new CachedJson(this.#entry, this);
    }
  }

  const cachedJson = (entry, response) =>
    new CachedJson(entry, { status: 200, statusText: "OK", headers: response.headers });

  async function fetchLiveMarkers(self, input, init) {
    const request = new Request(input, init);
    const key = request.url.split("#")[0];
    const last = liveMarkers.get(key);
    // a cross-origin live root would need a CORS preflight for this header
    if (last?.etag && new URL(key).origin === scope.location.origin) request.headers.set("If-None-Match", last.etag);
    const response = await plainFetch.call(self, request);
    if (response.status === 304 && last) return cachedJson(last, response);
    if (response.status !== 200) return response;
    const etag = response.headers.get("ETag");
    const text = await response.text();
    if (last && (last.text === text || (etag && etag === last.etag))) return cachedJson(last, response);
    let json;
    try {
      json = JSON.parse(text);
    } catch {
      return new Response(text, response);
    }
    // with an ETag the next answer is a 304 or a new body: no text to compare
    const entry = { json, etag, text: etag ? null : text };
    liveMarkers.set(key, entry);
    return cachedJson(entry, response);
  }
