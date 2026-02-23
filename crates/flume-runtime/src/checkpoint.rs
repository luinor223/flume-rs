//! Checkpoint subsystem: barrier alignment, coordination, and snapshot storage.

mod aligner;
mod coordinator;
mod store;

pub use aligner::{AlignerOutput, BarrierAligner};
pub use coordinator::CheckpointCoordinator;
pub use flume_core::CheckpointAck;
pub use store::SnapshotStore;
