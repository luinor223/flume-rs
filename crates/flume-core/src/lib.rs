//! Core types and traits for the flume-rs stream processing engine.
//!
//! This crate has minimal dependencies and defines the vocabulary
//! shared by every other crate in the workspace.

mod config;
mod error;
mod operator;
mod record;
mod time;
mod traits;

pub use config::PipelineConfig;
pub use error::{FlumeError, FlumeResult};
pub use operator::{FilterOperator, FlatMapOperator, MapOperator};
pub use record::{CheckpointBarrier, Record, StreamElement, Watermark};
pub use time::EventTimestamp;
pub use traits::{
    Collector, ListState, MapState, Operator, Sink, Source, StateBackend, ValueState,
};
