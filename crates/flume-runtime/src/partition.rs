//! Partitioning strategies for distributing records across subtasks.

use std::hash::Hash;

use ahash::RandomState;

/// Deterministic hash partitioning using fixed-seed AES-based hashing.
pub fn hash_partition<K: Hash>(key: &K, num_partitions: usize) -> usize {
    let state = RandomState::with_seeds(0, 0, 0, 0);
    (state.hash_one(key) as usize) % num_partitions
}

/// Partition strategy for routing records between subtasks.
pub enum Partitioner {
    /// 1:1 index mapping (same-index subtask).
    Forward,
    /// Route by `hash(key) % N`.
    Hash,
    /// Send copy to every downstream subtask.
    Broadcast,
    /// Round-robin across all downstream subtasks.
    Rebalance { next: usize },
}

impl Partitioner {
    /// Select target partition indices for a record.
    ///
    /// - `key`: the record's partition key (only used for `Hash`)
    /// - `source_index`: the subtask index of the sender
    /// - `num_targets`: total number of downstream subtasks
    pub fn select(
        &mut self,
        key: Option<&[u8]>,
        source_index: usize,
        num_targets: usize,
    ) -> Vec<usize> {
        match self {
            Partitioner::Forward => vec![source_index % num_targets],
            Partitioner::Hash => {
                let k = key.unwrap_or(&[]);
                vec![hash_partition(&k, num_targets)]
            }
            Partitioner::Broadcast => (0..num_targets).collect(),
            Partitioner::Rebalance { next } => {
                let target = *next % num_targets;
                *next = next.wrapping_add(1);
                vec![target]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_partition_deterministic() {
        let a = hash_partition(&"key-1", 4);
        let b = hash_partition(&"key-1", 4);
        assert_eq!(a, b);
    }

    #[test]
    fn test_hash_partition_in_range() {
        for i in 0..100 {
            let p = hash_partition(&i, 8);
            assert!(p < 8);
        }
    }

    #[test]
    fn test_forward_partitioner() {
        let mut p = Partitioner::Forward;
        assert_eq!(p.select(None, 2, 4), vec![2]);
        assert_eq!(p.select(None, 5, 4), vec![1]); // 5 % 4 = 1
    }

    #[test]
    fn test_broadcast_partitioner() {
        let mut p = Partitioner::Broadcast;
        assert_eq!(p.select(None, 0, 3), vec![0, 1, 2]);
    }

    #[test]
    fn test_rebalance_partitioner() {
        let mut p = Partitioner::Rebalance { next: 0 };
        assert_eq!(p.select(None, 0, 3), vec![0]);
        assert_eq!(p.select(None, 0, 3), vec![1]);
        assert_eq!(p.select(None, 0, 3), vec![2]);
        assert_eq!(p.select(None, 0, 3), vec![0]); // wraps around
    }

    #[test]
    fn test_hash_partitioner() {
        let mut p = Partitioner::Hash;
        let key = b"test-key";
        let result = p.select(Some(key), 0, 4);
        assert_eq!(result.len(), 1);
        assert!(result[0] < 4);
    }
}
