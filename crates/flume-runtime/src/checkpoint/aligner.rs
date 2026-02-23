//! Barrier aligner for Chandy-Lamport checkpoint alignment.
//!
//! For multi-input operators, buffers records on fast inputs until
//! checkpoint barriers arrive on all inputs before emitting the barrier.

use flume_core::{CheckpointBarrier, StreamElement};

/// Output produced by the barrier aligner.
#[derive(Debug)]
pub enum AlignerOutput<T> {
    /// A regular stream element to process.
    Element(StreamElement<T>),
    /// All inputs have received this barrier — time to snapshot state.
    BarrierAligned(CheckpointBarrier),
}

/// Aligns checkpoint barriers across multiple input channels.
pub struct BarrierAligner<T: Send + 'static> {
    _marker: std::marker::PhantomData<T>,
}

impl<T: Send + 'static> BarrierAligner<T> {
    pub fn new(_inputs: Vec<tokio::sync::mpsc::Receiver<StreamElement<T>>>) -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}
