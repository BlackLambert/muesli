//! Core of the Muesli engine.
//!
//! The core knows nothing about windows, graphics or ECS. It manages modules,
//! contracts, command queues, snapshots, the stage chain and the driver
//! interface.
//!
//! Chain rules:
//!
//! - **Queue rule:** A provider drains its queue at the start of its stage.
//!   Later commands take effect in the next run.
//! - **Snapshot rule:** After its stage a provider publishes its snapshot.
//!   Reading calls always see the most recently published state: stages after
//!   the provider see the new one, stages before it see the previous run's.
//! - Within a stage, all provider drains run first, then registered work, then
//!   all publishes, each with providers before their users.

mod contract;
mod driver;
mod error;
mod graph;
mod module;
mod runtime;
mod stage;

pub use contract::{Contract, ContractId, Drain, Handle, SnapshotRef};
pub use driver::{Driver, FixedRate, Once, Unbounded};
pub use error::{BoxError, BuildError, Error, RunError, StartError};
pub use module::{BuildCtx, DepCtx, DepSet, Module, Provider};
pub use runtime::{App, Flow, Runtime, StageCtx};
pub use stage::{Stage, StageOrder, Update};

/// Result with Muesli's error type.
pub type Result<T = (), E = Error> = std::result::Result<T, E>;
