//! Sink trait for data egress.

use std::future::Future;

use crate::{FlumeResult, Record};

/// Async trait for writing processed data to external systems.
pub trait Sink<T: Send + 'static>: Send {
    /// Write a single record to the external system.
    fn write(&mut self, record: Record<T>) -> impl Future<Output = FlumeResult<()>> + Send;

    /// Flush pending writes. Called before acknowledging a checkpoint barrier.
    fn flush(&mut self) -> impl Future<Output = FlumeResult<()>> + Send;

    /// Save the sink's state for checkpointing (e.g., two-phase commit).
    fn snapshot(&self) -> impl Future<Output = FlumeResult<Vec<u8>>> + Send;

    /// Restore from a previously checkpointed state.
    fn restore(&mut self, state: Vec<u8>) -> impl Future<Output = FlumeResult<()>> + Send;
}
