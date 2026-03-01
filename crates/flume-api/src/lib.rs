//! User-facing DataStream API for building stream processing pipelines.
//!
//! Mirrors Flink's DataStream API: a fluent builder pattern where each
//! transformation returns a new stream that can be further transformed.

pub mod config;
mod connected;
mod datastream;
mod environment;
pub mod functions;
mod interval;
mod join;
pub mod sink;
pub mod source;
mod windowed;

pub use connected::ConnectedStream;
pub use datastream::DataStream;
#[cfg(feature = "prometheus")]
pub use environment::init_metrics;
pub use environment::{StreamExecutionEnvironment, init_logging};
pub use interval::{BoundedIntervalJoinStream, IntervalJoinStream};
pub use join::{JoinStream, WindowedJoinStream};
pub use sink::{CollectSink, PrintSink};
pub use source::{InMemorySource, TimestampedSource};
pub use windowed::WindowedStream;
