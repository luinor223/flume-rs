//! Core types and traits for the flume-rs stream processing engine.
//!
//! This crate has minimal dependencies and defines the vocabulary
//! shared by every other crate in the workspace.

mod config;
mod error;
mod operator;
mod process;
mod record;
mod time;
mod timer;
mod traits;
mod window;

pub use config::{ChannelKind, PipelineConfig};
pub use error::{FlumeError, FlumeResult};
pub use operator::{
    FilterOperator, FlatMapOperator, KeyedProcessOperator, MapOperator, ProcessOperator,
    WindowOperator,
};
pub use process::{
    KeyedProcessFunction, OnTimerContext, OutputTag, ProcessContext, ProcessFunction, TimerService,
};
pub use record::{CheckpointAck, CheckpointBarrier, Record, StreamElement, Watermark};
pub use time::EventTimestamp;
pub use timer::{TimerQueue, TimerServiceImpl};
pub use traits::{
    Collector, ListState, MapState, Operator, Sink, Source, StateBackend, ValueState,
};
pub use window::{
    AggregateFunction, EventTimeTrigger, SlidingWindow, Trigger, TriggerResult, TumblingWindow,
    Window, WindowAssigner,
};
