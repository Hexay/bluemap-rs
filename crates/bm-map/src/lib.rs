//! BlueMap's `core.map` package: everything between the hires render and the stored map tiles. Everything persisted
//! here stays byte-compatible with Java BlueMap so a map can switch between the two (docs/00-overview.md).

pub mod lowres;
pub mod mask;
pub mod renderstate;
pub mod settings;
