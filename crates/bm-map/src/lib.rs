//! Per-map state of BlueMap (`core/map/**`). Everything persisted here stays byte-compatible with Java BlueMap so a
//! map can switch between the two implementations (docs/00-overview.md "Product goal").

pub mod renderstate;
