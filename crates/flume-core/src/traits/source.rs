//! Source trait for data ingestion.

use std::future::Future;

use crate::{FlumeResult, StreamElement};

/// Async trait for data ingestion. Sources are I/O-bound
/// (reading from Kafka, TCP, files).
///
/// Uses a manual async trait pattern instead of `async_trait` to avoid
/// the extra dependency in flume-core.
pub trait Source<T: Send + 'static>: Send {
    /// Fetch the next element. Returns `None` when the source is exhausted.
    fn next(&mut self) -> impl Future<Output = FlumeResult<Option<StreamElement<T>>>> + Send;

    /// Save the source's position (e.g., Kafka offsets) for checkpointing.
    fn snapshot(&self) -> impl Future<Output = FlumeResult<Vec<u8>>> + Send;

    /// Reset the source to a previously checkpointed position.
    fn restore(&mut self, state: Vec<u8>) -> impl Future<Output = FlumeResult<()>> + Send;
}
