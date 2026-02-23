//! In-memory state backend implementation.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;

use flume_core::{FlumeError, FlumeResult, ListState, MapState, StateBackend, ValueState};

use crate::erased::ErasedState;

// ---------------------------------------------------------------------------
// Shared storage types
// ---------------------------------------------------------------------------

type SharedValue<V> = Arc<Mutex<Option<V>>>;
type SharedList<V> = Arc<Mutex<Vec<V>>>;
type SharedMap<K, V> = Arc<Mutex<HashMap<K, V>>>;

// ---------------------------------------------------------------------------
// ValueState
// ---------------------------------------------------------------------------

/// Typed handle returned to operators.
pub(crate) struct MemoryValueState<V> {
    inner: SharedValue<V>,
}

impl<V: Send + Clone> ValueState<V> for MemoryValueState<V> {
    fn get(&self) -> FlumeResult<Option<V>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        Ok(guard.clone())
    }

    fn set(&mut self, value: V) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        *guard = Some(value);
        Ok(())
    }

    fn clear(&mut self) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        *guard = None;
        Ok(())
    }
}

/// Type-erased wrapper for snapshot/restore.
pub(crate) struct ErasedValueState<V> {
    inner: SharedValue<V>,
}

impl<V: Send + Clone + Serialize + DeserializeOwned> ErasedState for ErasedValueState<V> {
    fn snapshot_bytes(&self) -> FlumeResult<Vec<u8>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        bincode::serialize(&*guard).map_err(|e| FlumeError::Serialization(e.to_string()))
    }

    fn restore_bytes(&self, data: &[u8]) -> FlumeResult<()> {
        let value: Option<V> =
            bincode::deserialize(data).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        *guard = value;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ListState
// ---------------------------------------------------------------------------

pub(crate) struct MemoryListState<V> {
    inner: SharedList<V>,
}

impl<V: Send + Clone> ListState<V> for MemoryListState<V> {
    fn get(&self) -> FlumeResult<Vec<V>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        Ok(guard.clone())
    }

    fn add(&mut self, value: V) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        guard.push(value);
        Ok(())
    }

    fn clear(&mut self) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        guard.clear();
        Ok(())
    }
}

pub(crate) struct ErasedListState<V> {
    inner: SharedList<V>,
}

impl<V: Send + Clone + Serialize + DeserializeOwned> ErasedState for ErasedListState<V> {
    fn snapshot_bytes(&self) -> FlumeResult<Vec<u8>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        bincode::serialize(&*guard).map_err(|e| FlumeError::Serialization(e.to_string()))
    }

    fn restore_bytes(&self, data: &[u8]) -> FlumeResult<()> {
        let value: Vec<V> =
            bincode::deserialize(data).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        *guard = value;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MapState
// ---------------------------------------------------------------------------

pub(crate) struct MemoryMapState<K, V> {
    inner: SharedMap<K, V>,
}

impl<K: Send + Clone + Hash + Eq, V: Send + Clone> MapState<K, V> for MemoryMapState<K, V> {
    fn get(&self, key: &K) -> FlumeResult<Option<V>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        Ok(guard.get(key).cloned())
    }

    fn put(&mut self, key: K, value: V) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        guard.insert(key, value);
        Ok(())
    }

    fn remove(&mut self, key: &K) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        guard.remove(key);
        Ok(())
    }

    fn entries(&self) -> FlumeResult<HashMap<K, V>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        Ok(guard.clone())
    }

    fn clear(&mut self) -> FlumeResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        guard.clear();
        Ok(())
    }
}

pub(crate) struct ErasedMapState<K, V> {
    inner: SharedMap<K, V>,
}

impl<K, V> ErasedState for ErasedMapState<K, V>
where
    K: Send + Clone + Hash + Eq + Serialize + DeserializeOwned,
    V: Send + Clone + Serialize + DeserializeOwned,
{
    fn snapshot_bytes(&self) -> FlumeResult<Vec<u8>> {
        let guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        bincode::serialize(&*guard).map_err(|e| FlumeError::Serialization(e.to_string()))
    }

    fn restore_bytes(&self, data: &[u8]) -> FlumeResult<()> {
        let value: HashMap<K, V> =
            bincode::deserialize(data).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| FlumeError::State(e.to_string()))?;
        *guard = value;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MemoryStateBackend
// ---------------------------------------------------------------------------

/// In-memory implementation of [`StateBackend`].
///
/// Stores all state in `Arc<Mutex<...>>` so that both the returned handles
/// and the backend itself share access. Snapshot serializes all state to a
/// `HashMap<String, Vec<u8>>` via bincode.
pub struct MemoryStateBackend {
    states: HashMap<String, Box<dyn ErasedState>>,
}

impl MemoryStateBackend {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
        }
    }

    fn check_duplicate(&self, name: &str) -> FlumeResult<()> {
        if self.states.contains_key(name) {
            return Err(FlumeError::State(format!(
                "state '{}' already registered",
                name
            )));
        }
        Ok(())
    }
}

impl Default for MemoryStateBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StateBackend for MemoryStateBackend {
    fn create_value_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ValueState<V>>> {
        self.check_duplicate(name)?;
        let shared: SharedValue<V> = Arc::new(Mutex::new(None));
        self.states.insert(
            name.to_owned(),
            Box::new(ErasedValueState {
                inner: Arc::clone(&shared),
            }),
        );
        Ok(Box::new(MemoryValueState { inner: shared }))
    }

    fn create_list_state<V: Send + Clone + Serialize + DeserializeOwned + 'static>(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn ListState<V>>> {
        self.check_duplicate(name)?;
        let shared: SharedList<V> = Arc::new(Mutex::new(Vec::new()));
        self.states.insert(
            name.to_owned(),
            Box::new(ErasedListState {
                inner: Arc::clone(&shared),
            }),
        );
        Ok(Box::new(MemoryListState { inner: shared }))
    }

    fn create_map_state<
        K: Send + Clone + Hash + Eq + Serialize + DeserializeOwned + 'static,
        V: Send + Clone + Serialize + DeserializeOwned + 'static,
    >(
        &mut self,
        name: &str,
    ) -> FlumeResult<Box<dyn MapState<K, V>>> {
        self.check_duplicate(name)?;
        let shared: SharedMap<K, V> = Arc::new(Mutex::new(HashMap::new()));
        self.states.insert(
            name.to_owned(),
            Box::new(ErasedMapState {
                inner: Arc::clone(&shared),
            }),
        );
        Ok(Box::new(MemoryMapState { inner: shared }))
    }

    fn snapshot(&self) -> FlumeResult<Vec<u8>> {
        let mut map: HashMap<String, Vec<u8>> = HashMap::new();
        for (name, state) in &self.states {
            map.insert(name.clone(), state.snapshot_bytes()?);
        }
        bincode::serialize(&map).map_err(|e| FlumeError::Serialization(e.to_string()))
    }

    fn restore(&mut self, data: &[u8]) -> FlumeResult<()> {
        let map: HashMap<String, Vec<u8>> =
            bincode::deserialize(data).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        for (name, bytes) in &map {
            if let Some(state) = self.states.get(name) {
                state.restore_bytes(bytes)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_state_crud() {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_value_state::<i32>("counter").unwrap();

        assert_eq!(state.get().unwrap(), None);
        state.set(42).unwrap();
        assert_eq!(state.get().unwrap(), Some(42));
        state.set(100).unwrap();
        assert_eq!(state.get().unwrap(), Some(100));
        state.clear().unwrap();
        assert_eq!(state.get().unwrap(), None);
    }

    #[test]
    fn test_list_state_crud() {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_list_state::<String>("buffer").unwrap();

        assert!(state.get().unwrap().is_empty());
        state.add("a".into()).unwrap();
        state.add("b".into()).unwrap();
        assert_eq!(state.get().unwrap(), vec!["a", "b"]);
        state.clear().unwrap();
        assert!(state.get().unwrap().is_empty());
    }

    #[test]
    fn test_map_state_crud() {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_map_state::<String, i32>("lookup").unwrap();

        assert_eq!(state.get(&"x".into()).unwrap(), None);
        state.put("x".into(), 1).unwrap();
        state.put("y".into(), 2).unwrap();
        assert_eq!(state.get(&"x".into()).unwrap(), Some(1));

        let entries = state.entries().unwrap();
        assert_eq!(entries.len(), 2);

        state.remove(&"x".into()).unwrap();
        assert_eq!(state.get(&"x".into()).unwrap(), None);

        state.clear().unwrap();
        assert!(state.entries().unwrap().is_empty());
    }

    #[test]
    fn test_duplicate_name_errors() {
        let mut backend = MemoryStateBackend::new();
        backend.create_value_state::<i32>("counter").unwrap();
        assert!(backend.create_value_state::<i32>("counter").is_err());
        assert!(backend.create_list_state::<i32>("counter").is_err());
        assert!(backend.create_map_state::<String, i32>("counter").is_err());
    }

    #[test]
    fn test_snapshot_restore_roundtrip() {
        let mut backend = MemoryStateBackend::new();
        let mut val = backend.create_value_state::<i32>("counter").unwrap();
        let mut list = backend.create_list_state::<String>("buffer").unwrap();
        let mut map = backend.create_map_state::<String, i32>("lookup").unwrap();

        val.set(42).unwrap();
        list.add("hello".into()).unwrap();
        list.add("world".into()).unwrap();
        map.put("a".into(), 1).unwrap();

        let snapshot = backend.snapshot().unwrap();

        // Mutate state after snapshot
        val.set(999).unwrap();
        list.clear().unwrap();
        map.put("b".into(), 2).unwrap();

        // Restore should bring back snapshot state
        backend.restore(&snapshot).unwrap();

        assert_eq!(val.get().unwrap(), Some(42));
        assert_eq!(list.get().unwrap(), vec!["hello", "world"]);
        assert_eq!(map.get(&"a".into()).unwrap(), Some(1));
        assert_eq!(map.get(&"b".into()).unwrap(), None);
    }

    #[test]
    fn test_snapshot_restore_empty() {
        let mut backend = MemoryStateBackend::new();
        let val = backend.create_value_state::<i32>("counter").unwrap();

        let snapshot = backend.snapshot().unwrap();
        backend.restore(&snapshot).unwrap();

        assert_eq!(val.get().unwrap(), None);
    }
}
