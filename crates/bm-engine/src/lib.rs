//! BlueMap's render pipeline as a library, shared by the CLI and (later) the server plugin: a [`Service`] holds
//! the loaded config, resources and storages; [`Service::open_map`] prepares a map like `BmMap`'s constructor;
//! [`update_map`] runs a map update like BlueMap's region tasks, with output byte-compatible with Java BlueMap.
//! For long-running use, [`RenderQueue`] + [`run_queue`] are `RenderManager` and [`MapUpdateService`] watches a
//! map's region files and schedules updates.
//!
//! Differences from BlueMap's scheduling (never in output): each hires tile is processed by one region only,
//! render workers never wait on saves (a persistence thread owns lowres and render-state writes), and each
//! lowres tile is saved once its last contributing region is done.

mod actions;
mod addons;
mod convert;
mod error;
mod io;
mod job;
mod map;
mod mask;
mod persist;
mod plan;
mod queue;
mod render;
mod resources;
mod runner;
mod service;
mod storage;
mod task;
mod update;
mod watch;

pub use addons::{JavaAddon, find_java_addons};
pub use bm_map::renderstate::TileUpdateStrategy;
pub use error::{Error, Result};
pub use job::{Job, JobControl};
pub use map::{MapContext, TileListener};
pub use persist::PersistStats;
pub use queue::{PauseReason, PauseReasons, RenderQueue};
pub use resources::{ResourceOptions, Resources};
pub use runner::{LoadedMaps, TaskEvent, run_queue};
pub use service::{BLUEMAP_VERSION, Service};
pub use task::{Regions, RenderTask};
pub use update::{UpdateEvent, UpdateJob, UpdateStats, update_map};
pub use watch::{FullUpdates, LogFn, LogLevel, MapUpdateService, OnFullUpdate, WatchSettings};
