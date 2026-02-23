//! Sink trait for data egress.

use std::future::Future;
use std::pin::Pin;

use crate::{FlumeResult, Record};

/// Boxed future type alias for Sink methods.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async trait for writing processed data to external systems.
///
/// Uses boxed futures to allow dynamic dispatch (`Box<dyn Sink<T>>`).
pub trait Sink<T: Send + 'static>: Send {
    /// Write a single record to the external system.
    fn write(&mut self, record: Record<T>) -> BoxFuture<'_, FlumeResult<()>>;

    /// Flush pending writes. Called before acknowledging a checkpoint barrier.
    fn flush(&mut self) -> BoxFuture<'_, FlumeResult<()>>;

    /// Save the sink's state for checkpointing (e.g., two-phase commit).
    fn snapshot(&self) -> BoxFuture<'_, FlumeResult<Vec<u8>>>;

    /// Restore from a previously checkpointed state.
    fn restore(&mut self, state: Vec<u8>) -> BoxFuture<'_, FlumeResult<()>>;
}
