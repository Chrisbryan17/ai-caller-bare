//! Paper-first low-latency Polymarket copy-trading engine.
//! Live order submission is compile-time and runtime gated.

mod coordinator;
mod data_api;
mod discovery;
mod discovery_api;
mod dynamic_watchers;
mod execution;
mod journal;
mod launch;
mod model;
mod registry;
mod replay;
mod risk;
mod rotation;
mod runtime;
mod sizing;
mod strategy;
mod watcher;

#[cfg(feature = "live-trading")]
pub mod live;

pub use coordinator::*;
pub use data_api::*;
pub use discovery::*;
pub use discovery_api::*;
pub use dynamic_watchers::*;
pub use execution::*;
pub use journal::*;
pub use launch::*;
pub use model::*;
pub use registry::*;
pub use replay::*;
pub use risk::*;
pub use rotation::*;
pub use runtime::*;
pub use sizing::*;
pub use strategy::*;
pub use watcher::*;
