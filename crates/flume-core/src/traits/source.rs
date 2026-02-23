//! Source trait for data ingestion.

use std::future::Future;
use std::pin::Pin;

use crate::{FlumeResult, StreamElement};

/// Boxed future type alias for Source methods.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async trait for data ingestion. Sources are I/O-bound
/// (reading from Kafka, TCP, files).
///
/// Uses boxed futures to allow dynamic dispatch (`Box<dyn Source<T>>`).
pub trait Source<T: Send + 'static>: Send {
    /// Fetch the next element. Returns `None` when the source is exhausted.
    fn next(&mut self) -> BoxFuture<'_, FlumeResult<Option<StreamElement<T>>>>;

    /// Save the source's position (e.g., Kafka offsets) for checkpointing.
    fn snapshot(&self) -> BoxFuture<'_, FlumeResult<Vec<u8>>>;

    /// Reset the source to a previously checkpointed position.
    fn restore(&mut self, state: Vec<u8>) -> BoxFuture<'_, FlumeResult<()>>;
}
