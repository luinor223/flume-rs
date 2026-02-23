//! Barrier aligner stub.
//!
//! Planned behavior (Phase 6):
//! For multi-input operators, align checkpoint barriers across all inputs
//! using the Chandy-Lamport algorithm before snapshotting state.

/// Aligns checkpoint barriers across multiple input channels.
pub struct BarrierAligner {
    pub num_inputs: usize,
}

impl BarrierAligner {
    pub fn new(num_inputs: usize) -> Self {
        Self { num_inputs }
    }
}
