//! User-facing DataStream API for building stream processing pipelines.
//!
//! Mirrors Flink's DataStream API: a fluent builder pattern where each
//! transformation returns a new stream that can be further transformed.

mod datastream;
mod environment;
pub mod functions;
pub mod sink;
pub mod source;

pub use datastream::DataStream;
pub use environment::StreamExecutionEnvironment;
pub use sink::{CollectSink, PrintSink};
pub use source::InMemorySource;
