//! State management backends for the flume-rs stream processing engine.
//!
//! Provides [`MemoryStateBackend`] (in-memory, implements [`flume_core::StateBackend`])
//! and [`KeyedStateBackend`] (per-key state isolation with snapshot/restore).

mod erased;
pub mod keyed;
pub mod memory;

pub use keyed::KeyedStateBackend;
pub use memory::MemoryStateBackend;
