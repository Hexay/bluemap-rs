//! BlueMap's render pipeline as a library, shared by the CLI and (later) the server plugin: a [`Service`] holds
//! the loaded config, resources and storages; [`Service::open_map`] prepares a map like `BmMap`'s constructor;
//! [`update_map`] runs a map update like BlueMap's region tasks, with output byte-compatible with Java BlueMap.
//!
//! Differences from BlueMap's scheduling (never in output): each hires tile is processed by one region only,
//! render workers never wait on saves (a persistence thread owns lowres and render-state writes), and each
//! lowres tile is saved once its last contributing region is done.

mod actions;
mod convert;
mod error;
mod io;
mod map;
mod mask;
mod persist;
mod plan;
mod render;
mod resources;
mod service;
mod storage;
mod update;

pub use bm_map::renderstate::TileUpdateStrategy;
pub use error::{Error, Result};
pub use map::MapContext;
pub use persist::PersistStats;
pub use resources::{ResourceOptions, Resources};
pub use service::{BLUEMAP_VERSION, Service};
pub use update::{UpdateEvent, UpdateStats, update_map};
