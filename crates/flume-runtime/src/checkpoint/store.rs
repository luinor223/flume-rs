//! Snapshot store stub.
//!
//! Planned behavior (Phase 6):
//! - Write serialized state snapshots to `{base_path}/{checkpoint_id}/{operator_id}`
//! - Support listing available checkpoints for recovery
//! - Clean up old checkpoints after N successful ones

/// Manages checkpoint storage (local filesystem, later S3).
pub struct SnapshotStore {
    pub base_path: String,
}

impl SnapshotStore {
    pub fn new(base_path: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }
}
