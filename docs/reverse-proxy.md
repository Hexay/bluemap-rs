# Behind a reverse proxy: client addresses and rate limits

## Tell bluemap-rs who the client is

Behind nginx, Caddy or any other proxy every request comes from the proxy's address, so the `server-unpack` queue
(docs/18 "Clients that don't unpack") sees one client and can't share itself fairly. List the proxy in
`webserver.conf`:

```hocon
trusted-proxies: ["127.0.0.1", "::1"]
```

Entries are addresses or CIDR blocks (`10.0.0.0/8`, `fd00::/8`). For a request from a listed peer, the client is
the right-most `X-Forwarded-For` entry that isn't a listed proxy (or `X-Real-IP` when there is no
`X-Forwarded-For`). The header is ignored from any other peer, so a client can't pick its own address by sending
one. With a chain (CDN → nginx → bluemap-rs) list every hop the requests pass through, e.g. the CDN's published
ranges as well as nginx. IPv6 clients are counted per /64. Nothing else uses the setting: the access log still
writes what Java BlueMap writes.

## Rate-limit floods in front of the server

The `server-unpack` queue and its fair share only decide who gets the server's unpack slots. They don't stop a
flood: every request still costs an accepted connection, a parsed request and an answer, even when the answer is
a `503`. On the devbox 600 flooding connections got ~150k refusals per second out of bluemap-rs, using cores the
real viewers needed (docs/18 "A flood"). No limit inside the process can drop a request it has already been sent.
Drop it before it gets there: in the reverse proxy, or upstream of the host (a CDN, the provider's filtering) when
the flood saturates the proxy or the link too.

What a viewer needs: a view load is ~50 hires and ~16 lowres tiles at once (all at once over HTTP/2), and panning or
zooming loads another few dozen every second or two. Limit per client address, allow bursts of a few hundred, and
keep the sustained rate well above what a quickly panning viewer asks for. Limit only the tile paths: `index.html`,
assets and `live/` are small and cached.

### nginx

```nginx
# http { }
limit_req_zone  $binary_remote_addr zone=bluemap_tiles:10m rate=60r/s;
limit_conn_zone $binary_remote_addr zone=bluemap_inflight:10m;

server {
    listen 443 ssl;
    http2 on;
    server_name map.example.com;

    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Real-IP $remote_addr;

    location / {
        proxy_pass http://127.0.0.1:8100;
    }

    location ~ ^/maps/[^/]+/tiles/ {
        limit_req zone=bluemap_tiles burst=300 nodelay;
        # under HTTP/2 nginx counts each concurrent request here, so leave room for a whole view load
        limit_conn bluemap_inflight 128;
        limit_req_status 429;
        limit_conn_status 429;
        proxy_pass http://127.0.0.1:8100;
    }
}
```

`live/sse` streams through as is: bluemap-rs sends `X-Accel-Buffering: no`. Behind a CDN, set nginx's
`real_ip_header`/`set_real_ip_from` so `$binary_remote_addr` is the viewer and not the CDN.

### Caddy

Caddy has no built-in rate limiter; build it with the [caddy-ratelimit](https://github.com/mholt/caddy-ratelimit)
module (`xcaddy build --with github.com/mholt/caddy-ratelimit`). `reverse_proxy` sets `X-Forwarded-For` by itself.

```caddyfile
{
    order rate_limit before reverse_proxy
}

map.example.com {
    @tiles path_regexp ^/maps/[^/]+/tiles/
    rate_limit @tiles {
        zone bluemap_tiles {
            key {remote_host}
            events 300
            window 5s
        }
    }
    reverse_proxy 127.0.0.1:8100
}
```

Both answer clients over the limit with `429` before bluemap-rs sees the request. The webapp doesn't retry a
failed tile until it leaves the view distance, so set the limits for the busiest real viewer, not the average one.
