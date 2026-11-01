// view.mjs --dump: one JSON line per request of a phase, live data included. load_breakdown.mjs reads them.
import { appendFileSync } from "node:fs";

export function netDump(file) {
  const rows = new Map();
  let phase = "first";
  const row = (id) => {
    if (!rows.has(id)) rows.set(id, { phase, decoded: 0 });
    return rows.get(id);
  };
  return {
    startPhase(name) {
      this.flush();
      phase = name;
    },
    event(method, p) {
      if (method === "Network.requestWillBeSent") {
        if (!/^https?:/.test(p.request.url)) return;
        const frame = p.initiator?.stack?.callFrames?.[0];
        Object.assign(row(p.requestId), { url: p.request.url, type: p.type, start: p.timestamp, initiator: p.initiator?.type,
          caller: frame && `${frame.functionName}@${frame.lineNumber}:${frame.columnNumber}` });
      } else if (!rows.has(p.requestId) && method !== "Network.requestWillBeSentExtraInfo") {
        return;
      } else if (method === "Network.requestWillBeSentExtraInfo") {
        row(p.requestId).requestHeaders = p.headers;
      } else if (method === "Network.responseReceived") {
        const r = p.response;
        Object.assign(row(p.requestId), { status: r.status, mime: r.mimeType, protocol: r.protocol, reused: r.connectionReused,
          headerBytes: r.encodedDataLength, fromDiskCache: r.fromDiskCache, responseHeaders: r.headers });
      } else if (method === "Network.requestServedFromCache") {
        row(p.requestId).servedFromCache = true;
      } else if (method === "Network.dataReceived") {
        row(p.requestId).decoded += p.dataLength;
      } else if (method === "Network.loadingFinished") {
        Object.assign(row(p.requestId), { end: p.timestamp, bytes: p.encodedDataLength });
      } else if (method === "Network.loadingFailed") {
        Object.assign(row(p.requestId), { end: p.timestamp, failed: p.errorText, canceled: p.canceled });
      }
    },
    flush() {
      for (const r of rows.values()) if (r.url) appendFileSync(file, JSON.stringify(r) + "\n");
      rows.clear();
    },
  };
}
