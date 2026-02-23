//! State traits for keyed stream processing.

use std::collections::HashMap;
use std::hash::Hash;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::FlumeResult;

/// Single value per key. Use for counters, last-seen values.
pub trait ValueState<V: Send + Clone>: Send {
    fn get(&self) -> FlumeResult<Option<V>>;
    fn set(&mut self, value: V) -> FlumeResult<()>;
    fn clear(&mut self) -> FlumeResult<()>;
}

/// Append-optimized list per key. Use for event buffers, window contents.
pub trait ListState<V: Send + Clone>: Send {
    fn get(&self) -> FlumeResult<Vec<V>>;
    fn add(&mut self, value: V) -> FlumeResult<()>;
    fn clear(&mut self) -> FlumeResult<()>;
}

/// Per-entry access map per key. Use for lookup tables, indexes.
pub trait MapState<K: Send + Hash + Eq, V: Send + Clone>: Send {
    fn get(&self, key: &K) -> FlumeResult<Option<V>>;
    fn put(&mut self, key: K, value: V) -> FlumeResult<()>;
    fn remove(&mut self, key: &K) -> FlumeResult<()>;
    fn entries(&self) -> FlumeResult<HashMap<K, V>>;
    fn clear(&mut self) -> FlumeResult<()>;
}

/// Factory that creates state instances and manages snapshots.
pub trait StateBackend: Send {
    fn create_value_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ValueState<V>>>;

    fn create_list_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ListState<V>>>;

    fn create_map_state<
        K: Send + Clone + Hash + Eq + Serialize + DeserializeOwned + 'static,
        V: Send + Clone + Serialize + DeserializeOwned + 'static,
    >(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn MapState<K, V>>>;

    /// Serialize all state for checkpointing.
    fn snapshot(&self) -> FlumeResult<Vec<u8>>;

    /// Restore all state from a checkpoint.
    fn restore(&mut self, data: &[u8]) -> FlumeResult<()>;
}
