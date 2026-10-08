# Contributing

Thanks for helping. Bug reports, parity differences against Java BlueMap and pull requests are all welcome.

## The one rule: drop-in beats clean

bluemap-rs replaces BlueMap 5.28 without users changing anything. Configs, paths, tile files (PRBM, PNG, JSON), render
state, the SQL schema, command output and the webapp's view of the data stay byte-compatible with upstream, even when
a cleaner design is obvious. Copy BlueMap's visual quirks; never copy its robustness bugs (crashes, leaks, hangs; see
`docs/08-github-issues.md`). Deliberate deviations are listed in `MIGRATING.md` and `docs/15-beyond-parity.md`.

When upstream's behaviour is unclear, read its source (BlueMap 5.28) rather than guess. `docs/00-overview.md` is the
map of the design docs.

## Setup

- Rust 1.94+ (stable), Python 3.11+ (stdlib only) for `tools/`, JDK 25 for the Paper/Fabric shims in `platforms/`.
- `cargo test --workspace` needs nothing else. The first build compiles zstd's and libdeflate's C code.
- Golden and acceptance tests compare against Java BlueMap's own output, which is generated locally, not committed:

  ```sh
  py -3 tools/setup.py                 # JDK, Minecraft server jar, BlueMap CLI → work/downloads
  py -3 tools/make_world.py vanilla    # fixture world from fixtures/vanilla → work/worlds
  py -3 tools/render_golden.py vanilla # Java BlueMap's render → work/bluemap/vanilla
  cargo test -p bm-render --release    # our renders vs Java's, every face
  py -3 tools/accept.py vanilla        # our CLI on Java's config folder, webroot vs Java's
  ```

  Tests that need these fixtures skip when `work/` lacks them. Use `python3` instead of `py -3` outside Windows.
- Plugin/mod end-to-end tests start real servers: `py -3 tools/e2e_paper.py`, `py -3 tools/e2e_fabric.py`.

`CLAUDE.md` lists every crate and command in one place.

## Code

- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must pass (CI runs both on
  Linux and Windows, plus cargo-deny, a static musl build and the Gradle build).
- Format with `cargo fmt` (`rustfmt.toml`, 120 columns).
- Library crates return typed errors (`thiserror`); `anyhow` only in binaries and `bm-golden`.
- Hot paths write into caller-provided buffers (`*_into` functions) instead of returning fresh allocations.
- Comments explain what the code can't: a non-obvious reason, a gotcha, an upstream quirk being copied (cite the Java
  class and line). No comments that restate the code.
- A render change needs a golden diff: `cargo run -p bm-golden -- diff-render <java-webroot> <our-webroot>` reports
  every differing face. A performance change needs before/after numbers (`tools/bench.py`).

## Pull requests

- One topic per PR, with a description of what changed and how you checked it.
- New behaviour comes with a test. For parity fixes, a test that fails before the fix.
- By contributing you agree your work is released under the [MIT license](LICENSE).

## Reporting bugs

Use the issue templates. For a server, `/bluemap debug dump` writes a `dump.json` with the config and state that
helps most; check it for anything private before attaching it. If the same setup renders correctly with Java BlueMap
5.28, say so: that makes it a parity bug, which we treat as high priority.
