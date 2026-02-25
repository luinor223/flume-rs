//! Single-node execution engine for flume-rs.
//!
//! Takes the logical graph built by `flume-api` and runs it as a set
//! of concurrent tasks connected by bounded channels.

pub mod channel;
pub mod checkpoint;
pub mod dag;
pub mod partition;
pub mod physical;
pub mod pinned;
pub mod ring_buffer;
pub mod scheduler;
pub mod task;
pub mod test_harness;
pub mod watermark;
