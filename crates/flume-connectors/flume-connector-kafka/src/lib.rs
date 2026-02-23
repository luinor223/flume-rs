//! Kafka connector for the flume-rs stream processing engine.
//!
//! Provides Kafka source and sink using `rdkafka`.

mod sink;
mod source;

pub use sink::{KafkaSink, KafkaSinkBuilder};
pub use source::{KafkaSource, KafkaSourceBuilder};
