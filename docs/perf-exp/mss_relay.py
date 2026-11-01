#!/usr/bin/env python3
"""usage: mss_relay.py <listen port> <upstream port> [mss=1448]

Loopback TCP relay whose upstream sockets advertise a real network's MSS, so a server sends segments as it would over
Ethernet (no root needed; netem would need it). No RTT or bandwidth limit: add those with view.mjs --net.
"""
import asyncio
import socket
import sys


async def pump(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except ConnectionError:
        pass
    finally:
        writer.close()


async def main():
    listen, upstream = int(sys.argv[1]), int(sys.argv[2])
    mss = int(sys.argv[3]) if len(sys.argv) > 3 else 1448

    async def handle(reader, writer):
        s = socket.socket()
        s.setblocking(False)
        # gotcha: only takes effect before connect
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_MAXSEG, mss)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        await asyncio.get_running_loop().sock_connect(s, ("127.0.0.1", upstream))
        up_reader, up_writer = await asyncio.open_connection(sock=s)
        await asyncio.gather(pump(reader, up_writer), pump(up_reader, writer))

    server = await asyncio.start_server(handle, "127.0.0.1", listen)
    async with server:
        await server.serve_forever()


asyncio.run(main())
