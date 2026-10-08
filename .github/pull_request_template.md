## What and why

## How it was checked

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`
- [ ] Render or format change: golden diff against Java BlueMap (`bm-golden diff-render`) or acceptance (`tools/accept.py`)
- [ ] Plugin/mod change: `tools/e2e_paper.py` / `tools/e2e_fabric.py`
- [ ] Deliberate difference from BlueMap 5.28 documented in `MIGRATING.md` / `docs/15-beyond-parity.md`
