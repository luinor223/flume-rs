//! Flume server — HTTP-based job management for the flume-rs streaming engine.

pub mod job;
pub mod registry;

pub use job::{JobHandle, JobStatus};
pub use registry::JobRegistry;
