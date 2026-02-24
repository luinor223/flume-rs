//! Property-based tests for state backend snapshot/restore roundtrips
//! and raw bitcode serialization.

use flume_core::StateBackend;
use flume_state::MemoryStateBackend;
use proptest::prelude::*;

proptest! {
    #[test]
    fn value_state_snapshot_restore_roundtrip(value in any::<i64>()) {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_value_state::<i64>("v").unwrap();
        state.set(value).unwrap();

        let snapshot = backend.snapshot().unwrap();
        state.set(0).unwrap(); // mutate

        backend.restore(&snapshot).unwrap();
        prop_assert_eq!(state.get().unwrap(), Some(value));
    }

    #[test]
    fn list_state_snapshot_restore_roundtrip(values in prop::collection::vec(any::<i64>(), 0..50)) {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_list_state::<i64>("l").unwrap();
        for v in &values {
            state.add(*v).unwrap();
        }

        let snapshot = backend.snapshot().unwrap();
        state.clear().unwrap();

        backend.restore(&snapshot).unwrap();
        prop_assert_eq!(state.get().unwrap(), values);
    }

    #[test]
    fn map_state_snapshot_restore_roundtrip(entries in prop::collection::hash_map("\\PC{1,10}", any::<i64>(), 0..20)) {
        let mut backend = MemoryStateBackend::new();
        let mut state = backend.create_map_state::<String, i64>("m").unwrap();
        for (k, v) in &entries {
            state.put(k.clone(), *v).unwrap();
        }

        let snapshot = backend.snapshot().unwrap();
        state.clear().unwrap();

        backend.restore(&snapshot).unwrap();
        let restored = state.entries().unwrap();
        prop_assert_eq!(restored, entries);
    }

    #[test]
    fn empty_state_snapshot_restore(dummy in 0..1i32) {
        let _ = dummy;
        let mut backend = MemoryStateBackend::new();
        let state = backend.create_value_state::<i64>("empty").unwrap();

        let snapshot = backend.snapshot().unwrap();
        backend.restore(&snapshot).unwrap();
        prop_assert_eq!(state.get().unwrap(), None);
    }

    #[test]
    fn raw_bitcode_i64_roundtrip(value in any::<i64>()) {
        let bytes = bitcode::serialize(&value).unwrap();
        let restored: i64 = bitcode::deserialize(&bytes).unwrap();
        prop_assert_eq!(value, restored);
    }

    #[test]
    fn raw_bitcode_string_roundtrip(value in "\\PC{0,100}") {
        let bytes = bitcode::serialize(&value).unwrap();
        let restored: String = bitcode::deserialize(&bytes).unwrap();
        prop_assert_eq!(value, restored);
    }

    #[test]
    fn raw_bitcode_vec_roundtrip(value in prop::collection::vec(any::<i64>(), 0..100)) {
        let bytes = bitcode::serialize(&value).unwrap();
        let restored: Vec<i64> = bitcode::deserialize(&bytes).unwrap();
        prop_assert_eq!(value, restored);
    }
}
