//! Paper-first low-latency Polymarket copy-trading engine.
//! Live order submission is compile-time and runtime gated.

mod coordinator;
mod data_api;
mod execution;
mod journal;
mod model;
mod risk;
mod sizing;
mod strategy;

#[cfg(feature = "live-trading")]
pub mod live;

pub use coordinator::*;
pub use data_api::*;
pub use execution::*;
pub use journal::*;
pub use model::*;
pub use risk::*;
pub use sizing::*;
pub use strategy::*;
