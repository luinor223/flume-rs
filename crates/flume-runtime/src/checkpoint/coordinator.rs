//! Checkpoint coordinator stub.
//!
//! Planned behavior (Phase 6):
//! 1. Periodically inject `CheckpointBarrier` into every source
//! 2. Track which operators have acknowledged each barrier
//! 3. When all operators acknowledge, mark checkpoint as complete

/// Coordinates distributed checkpoints across the pipeline.
pub struct CheckpointCoordinator {
    pub next_checkpoint_id: u64,
}

impl CheckpointCoordinator {
    pub fn new() -> Self {
        Self {
            next_checkpoint_id: 0,
        }
    }
}

impl Default for CheckpointCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
