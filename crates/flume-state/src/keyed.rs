//! Keyed state backend — maintains one [`MemoryStateBackend`] per key.

use std::collections::HashMap;
use std::hash::Hash;

use serde::Serialize;
use serde::de::DeserializeOwned;

use flume_core::{FlumeError, FlumeResult, ListState, MapState, StateBackend, ValueState};

use crate::memory::MemoryStateBackend;

/// Wraps [`MemoryStateBackend`], maintaining a separate backend per serialized key.
///
/// Call [`set_current_key`](KeyedStateBackend::set_current_key) before creating
/// or accessing state. All state operations are scoped to the current key.
pub struct KeyedStateBackend {
    backends: HashMap<Vec<u8>, MemoryStateBackend>,
    current_key: Option<Vec<u8>>,
}

impl KeyedStateBackend {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
            current_key: None,
        }
    }

    /// Set the current key. State operations will be scoped to this key.
    pub fn set_current_key<K: Serialize>(&mut self, key: &K) -> FlumeResult<()> {
        let bytes =
            bincode::serialize(key).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        self.current_key = Some(bytes);
        Ok(())
    }

    /// Clear the current key.
    pub fn clear_current_key(&mut self) {
        self.current_key = None;
    }

    fn current_backend(&mut self) -> FlumeResult<&mut MemoryStateBackend> {
        let key = self
            .current_key
            .as_ref()
            .ok_or_else(|| FlumeError::State("no current key set".into()))?
            .clone();
        Ok(self.backends.entry(key).or_default())
    }

    /// Snapshot all keyed state: `HashMap<Vec<u8>, Vec<u8>>` (key → per-backend bytes).
    pub fn snapshot(&self) -> FlumeResult<Vec<u8>> {
        let mut map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for (key, backend) in &self.backends {
            map.insert(key.clone(), backend.snapshot()?);
        }
        bincode::serialize(&map).map_err(|e| FlumeError::Serialization(e.to_string()))
    }

    /// Restore all keyed state from snapshot bytes.
    pub fn restore(&mut self, data: &[u8]) -> FlumeResult<()> {
        let map: HashMap<Vec<u8>, Vec<u8>> =
            bincode::deserialize(data).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        for (key, bytes) in &map {
            if let Some(backend) = self.backends.get_mut(key) {
                backend.restore(bytes)?;
            }
        }
        Ok(())
    }

    /// Create a value state scoped to the current key.
    pub fn create_value_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ValueState<V>>> {
        self.current_backend()?.create_value_state(name)
    }

    /// Create a list state scoped to the current key.
    pub fn create_list_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ListState<V>>> {
        self.current_backend()?.create_list_state(name)
    }

    /// Create a map state scoped to the current key.
    pub fn create_map_state<
        K: Send + Clone + Hash + Eq + Serialize + DeserializeOwned + 'static,
        V: Send + Clone + Serialize + DeserializeOwned + 'static,
    >(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn MapState<K, V>>> {
        self.current_backend()?.create_map_state(name)
    }
}

impl Default for KeyedStateBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyed_isolation() {
        let mut keyed = KeyedStateBackend::new();

        keyed.set_current_key(&"user_a").unwrap();
        let mut val_a = keyed.create_value_state::<i32>("counter").unwrap();
        val_a.set(10).unwrap();

        keyed.set_current_key(&"user_b").unwrap();
        let mut val_b = keyed.create_value_state::<i32>("counter").unwrap();
        val_b.set(20).unwrap();

        // Each key has its own state
        assert_eq!(val_a.get().unwrap(), Some(10));
        assert_eq!(val_b.get().unwrap(), Some(20));
    }

    #[test]
    fn test_no_current_key_errors() {
        let mut keyed = KeyedStateBackend::new();
        assert!(keyed.create_value_state::<i32>("counter").is_err());
    }

    #[test]
    fn test_clear_current_key() {
        let mut keyed = KeyedStateBackend::new();
        keyed.set_current_key(&"key").unwrap();
        keyed.clear_current_key();
        assert!(keyed.create_value_state::<i32>("counter").is_err());
    }

    #[test]
    fn test_keyed_snapshot_restore() {
        let mut keyed = KeyedStateBackend::new();

        keyed.set_current_key(&"k1").unwrap();
        let mut v1 = keyed.create_value_state::<i32>("val").unwrap();
        v1.set(100).unwrap();

        keyed.set_current_key(&"k2").unwrap();
        let mut v2 = keyed.create_value_state::<i32>("val").unwrap();
        v2.set(200).unwrap();

        let snapshot = keyed.snapshot().unwrap();

        v1.set(999).unwrap();
        v2.set(888).unwrap();

        keyed.restore(&snapshot).unwrap();

        assert_eq!(v1.get().unwrap(), Some(100));
        assert_eq!(v2.get().unwrap(), Some(200));
    }
}
