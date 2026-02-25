//! Flume server — HTTP-based job management for the flume-rs streaming engine.

pub mod api;
pub mod cli;
pub mod config;
pub mod jm;
pub mod job;
pub mod registry;
pub mod tm;

pub use job::{JobHandle, JobStatus};
pub use registry::JobRegistry;
