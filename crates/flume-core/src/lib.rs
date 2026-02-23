//! Core types and traits for the flume-rs stream processing engine.
//!
//! This crate has minimal dependencies and defines the vocabulary
//! shared by every other crate in the workspace.

mod time;

pub use time::EventTimestamp;
