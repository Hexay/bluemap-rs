# Security policy

## Supported versions

bluemap-rs is pre-1.0. Only the latest release gets security fixes.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through GitHub:
**Security → Report a vulnerability** on this repository. You should get an answer within a week.

Include the version (`bluemap --version`, or `/bluemap version` on a server), the platform (CLI, Paper, Fabric), and
steps or a request that reproduces the problem.

## What is in scope

- **The built-in webserver** (`webserver.conf`). It is often exposed to the internet, so anything reachable over HTTP
  counts: path traversal out of the webroot or a map's storage, reading files that should not be served, crashes or
  unbounded memory/CPU from crafted requests, header injection.
- **Storage**: SQL injection through map ids or other config values, credentials leaking into logs or the webapp.
- **The plugin/mod shim**: the Java side talks to the Rust core only over the child process's stdin/stdout, so it
  opens no network port. A way for another local user or a player to drive the core counts.
- **Release artifacts**: anything wrong with the published binaries or jars.

## Out of scope

- Bugs in BlueMap itself (report those [upstream](https://github.com/BlueMap-Minecraft/BlueMap)), unless bluemap-rs
  copied them. We copy BlueMap's visual quirks on purpose but not its robustness bugs.
- The webapp's own code (we ship BlueMap 5.28's webapp unchanged); report webapp issues upstream too.
- TLS. The webserver speaks plain HTTP like BlueMap's. Put it behind a reverse proxy for HTTPS.
- Denial of service that needs write access to the server's world or config folders.
