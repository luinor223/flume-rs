//! Filesystem-based checkpoint snapshot storage.
//!
//! Layout: `{base_path}/cp-{checkpoint_id}/{operator_name}.bin`
//! A `_complete` marker file indicates a committed checkpoint.

use std::path::PathBuf;

use flume_core::{FlumeError, FlumeResult};

/// Manages checkpoint storage on the local filesystem.
pub struct SnapshotStore {
    base_path: PathBuf,
}

impl SnapshotStore {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Directory for a specific checkpoint.
    fn checkpoint_dir(&self, checkpoint_id: u64) -> PathBuf {
        self.base_path.join(format!("cp-{checkpoint_id}"))
    }

    /// Write operator state bytes for a checkpoint.
    pub async fn save_state(
        &self,
        checkpoint_id: u64,
        operator_name: &str,
        data: &[u8],
    ) -> FlumeResult<()> {
        let dir = self.checkpoint_dir(checkpoint_id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(FlumeError::Io)?;
        let path = dir.join(format!("{operator_name}.bin"));
        tokio::fs::write(&path, data)
            .await
            .map_err(FlumeError::Io)?;
        Ok(())
    }

    /// Mark a checkpoint as committed by writing a `_complete` marker.
    pub async fn commit(&self, checkpoint_id: u64) -> FlumeResult<()> {
        let marker = self.checkpoint_dir(checkpoint_id).join("_complete");
        tokio::fs::write(&marker, b"")
            .await
            .map_err(FlumeError::Io)?;
        Ok(())
    }

    /// Read operator state bytes from a checkpoint.
    pub async fn load_state(
        &self,
        checkpoint_id: u64,
        operator_name: &str,
    ) -> FlumeResult<Vec<u8>> {
        let path = self
            .checkpoint_dir(checkpoint_id)
            .join(format!("{operator_name}.bin"));
        tokio::fs::read(&path).await.map_err(FlumeError::Io)
    }

    /// Find the highest committed checkpoint ID, or `None` if none exist.
    pub async fn latest_checkpoint(&self) -> FlumeResult<Option<u64>> {
        let mut best: Option<u64> = None;
        let mut entries = match tokio::fs::read_dir(&self.base_path).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(FlumeError::Io(e)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(FlumeError::Io)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id_str) = name.strip_prefix("cp-")
                && let Ok(id) = id_str.parse::<u64>()
            {
                let marker = entry.path().join("_complete");
                if tokio::fs::metadata(&marker).await.is_ok() {
                    best = Some(best.map_or(id, |b: u64| b.max(id)));
                }
            }
        }
        Ok(best)
    }

    /// Remove old checkpoints, keeping only the most recent `keep_last_n`.
    pub async fn cleanup(&self, keep_last_n: usize) -> FlumeResult<()> {
        let mut committed: Vec<u64> = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.base_path).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(FlumeError::Io(e)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(FlumeError::Io)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id_str) = name.strip_prefix("cp-")
                && let Ok(id) = id_str.parse::<u64>()
            {
                let marker = entry.path().join("_complete");
                if tokio::fs::metadata(&marker).await.is_ok() {
                    committed.push(id);
                }
            }
        }
        committed.sort_unstable();
        if committed.len() > keep_last_n {
            let to_remove = committed.len() - keep_last_n;
            for &id in &committed[..to_remove] {
                let dir = self.checkpoint_dir(id);
                let _ = tokio::fs::remove_dir_all(&dir).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path());

        store.save_state(1, "op_a", b"hello").await.unwrap();
        store.save_state(1, "op_b", b"world").await.unwrap();

        let a = store.load_state(1, "op_a").await.unwrap();
        let b = store.load_state(1, "op_b").await.unwrap();
        assert_eq!(a, b"hello");
        assert_eq!(b, b"world");
    }

    #[tokio::test]
    async fn test_commit_and_latest() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path());

        // No checkpoints yet
        assert_eq!(store.latest_checkpoint().await.unwrap(), None);

        // Save but don't commit
        store.save_state(1, "op", b"data").await.unwrap();
        assert_eq!(store.latest_checkpoint().await.unwrap(), None);

        // Commit checkpoint 1
        store.commit(1).await.unwrap();
        assert_eq!(store.latest_checkpoint().await.unwrap(), Some(1));

        // Add checkpoint 3 (committed) — should be latest
        store.save_state(3, "op", b"data3").await.unwrap();
        store.commit(3).await.unwrap();
        assert_eq!(store.latest_checkpoint().await.unwrap(), Some(3));

        // Add checkpoint 2 (uncommitted) — latest still 3
        store.save_state(2, "op", b"data2").await.unwrap();
        assert_eq!(store.latest_checkpoint().await.unwrap(), Some(3));
    }

    #[tokio::test]
    async fn test_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path());

        for id in 1..=5 {
            store
                .save_state(id, "op", format!("data-{id}").as_bytes())
                .await
                .unwrap();
            store.commit(id).await.unwrap();
        }

        // Keep only last 2
        store.cleanup(2).await.unwrap();

        // Checkpoints 1, 2, 3 should be gone
        assert!(store.load_state(1, "op").await.is_err());
        assert!(store.load_state(2, "op").await.is_err());
        assert!(store.load_state(3, "op").await.is_err());

        // Checkpoints 4 and 5 should remain
        assert_eq!(store.load_state(4, "op").await.unwrap(), b"data-4");
        assert_eq!(store.load_state(5, "op").await.unwrap(), b"data-5");
    }

    #[tokio::test]
    async fn test_latest_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let non_existent = dir.path().join("does-not-exist");
        let store = SnapshotStore::new(non_existent);
        assert_eq!(store.latest_checkpoint().await.unwrap(), None);
    }
}
