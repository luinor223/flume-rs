//! Type-erased state trait for snapshot/restore without knowing concrete types.

use flume_core::FlumeResult;

/// Type-erased state handle for checkpointing.
///
/// Each concrete state type (`MemoryValueState`, `MemoryListState`, `MemoryMapState`)
/// has a corresponding erased wrapper that implements this trait. The backend stores
/// `Box<dyn ErasedState>` and can serialize all state without knowing concrete types.
pub(crate) trait ErasedState: Send {
    /// Serialize the current state to bytes (bincode).
    fn snapshot_bytes(&self) -> FlumeResult<Vec<u8>>;

    /// Restore state from bytes (bincode).
    fn restore_bytes(&self, data: &[u8]) -> FlumeResult<()>;
}
