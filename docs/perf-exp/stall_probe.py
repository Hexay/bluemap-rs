#!/usr/bin/env python3
"""usage: stall_probe.py <host:port> <urls file> [--mss N] [--n 300]

Per-response latency on one keep-alive connection, optionally with a clamped TCP MSS (a real network's segment size,
on loopback, no root needed). Shows whether Java's Nagle/delayed-ACK stall (docs/results/devbox-web-java-vs-rs-*)
depends on loopback's 64 KB MSS. Prints one JSON line.
"""
import argparse
import http.client
import json
import socket
import statistics
import time


class Conn(http.client.HTTPConnection):
    mss = None

    def connect(self):
        self.sock = socket.socket()
        if self.mss:
            # gotcha: only takes effect before connect
            self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_MAXSEG, self.mss)
        self.sock.connect((self.host, self.port))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("addr")
    p.add_argument("urls")
    p.add_argument("--mss", type=int)
    p.add_argument("--n", type=int, default=300)
    a = p.parse_args()
    host, port = a.addr.split(":")
    urls = open(a.urls).read().split()[: a.n]
    conn = Conn(host, int(port))
    conn.mss = a.mss
    conn.connect()
    seen_mss = conn.sock.getsockopt(socket.IPPROTO_TCP, socket.TCP_MAXSEG)
    ms = []
    for url in urls:
        t = time.perf_counter()
        conn.request("GET", "/" + url.lstrip("/"), headers={"Accept-Encoding": "gzip, deflate, br, zstd"})
        conn.getresponse().read()
        ms.append((time.perf_counter() - t) * 1e3)
    rest = sorted(ms[1:])  # the first response on a connection never stalls
    print(json.dumps({
        "mss_asked": a.mss, "mss_seen": seen_mss, "n": len(rest), "p50_ms": round(statistics.median(rest), 2),
        "p90_ms": round(rest[len(rest) * 9 // 10], 2), "over_30ms": sum(x > 30 for x in rest), "first_ms": round(ms[0], 2),
    }))


main()
