//! Checkpoint subsystem stubs (Phase 6 implementation).

mod aligner;
mod coordinator;
mod store;

pub use aligner::BarrierAligner;
pub use coordinator::CheckpointCoordinator;
pub use store::SnapshotStore;
