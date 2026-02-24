//! Checkpoint coordinator: periodically injects barriers, collects acks,
//! persists state snapshots, and cleans up old checkpoints.

use std::collections::HashSet;
use std::time::Duration;

use flume_core::{CheckpointAck, CheckpointBarrier, EventTimestamp};
use metrics::{counter, histogram};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::SnapshotStore;

/// Coordinates checkpoints across a single-node pipeline.
///
/// Runs as a tokio task that:
/// 1. Periodically sends `CheckpointBarrier` to all source channels
/// 2. Collects `CheckpointAck` from all operators
/// 3. Persists state to `SnapshotStore` and commits
/// 4. Cleans up old checkpoints
pub struct CheckpointCoordinator {
    next_checkpoint_id: u64,
    barrier_senders: Vec<mpsc::Sender<CheckpointBarrier>>,
    ack_receiver: mpsc::Receiver<CheckpointAck>,
    expected_operators: Vec<String>,
    store: SnapshotStore,
    interval: Duration,
    timeout: Duration,
    max_retained: usize,
}

impl CheckpointCoordinator {
    pub fn new(
        barrier_senders: Vec<mpsc::Sender<CheckpointBarrier>>,
        ack_receiver: mpsc::Receiver<CheckpointAck>,
        expected_operators: Vec<String>,
        store: SnapshotStore,
        interval: Duration,
        timeout: Duration,
        max_retained: usize,
    ) -> Self {
        Self {
            next_checkpoint_id: 1,
            barrier_senders,
            ack_receiver,
            expected_operators,
            store,
            interval,
            timeout,
            max_retained,
        }
    }

    /// Run the coordinator loop. Returns when all barrier senders are closed
    /// (i.e., the pipeline has shut down) or the cancellation token is triggered.
    ///
    /// On cancellation, triggers one final checkpoint before returning.
    pub async fn run(&mut self, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(self.interval);
        // The first tick fires immediately; skip it so the pipeline can start.
        ticker.tick().await;

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    info!("checkpoint coordinator cancelled, triggering final checkpoint");
                    let cp_id = self.next_checkpoint_id;
                    self.next_checkpoint_id += 1;
                    let _ = self.trigger_and_collect(cp_id).await;
                    return;
                }
                _ = ticker.tick() => {}
            }

            let cp_id = self.next_checkpoint_id;
            self.next_checkpoint_id += 1;

            info!(checkpoint_id = cp_id, "starting checkpoint");
            let start = std::time::Instant::now();
            match self.trigger_and_collect(cp_id).await {
                CheckpointResult::Success => {
                    let duration_ms = start.elapsed().as_millis() as f64;
                    counter!("flume.checkpoints.completed").increment(1);
                    histogram!("flume.checkpoint.duration_ms").record(duration_ms);
                    info!(checkpoint_id = cp_id, "checkpoint completed");
                    let _ = self.store.cleanup(self.max_retained).await;
                }
                CheckpointResult::Timeout => {
                    counter!("flume.checkpoints.timed_out").increment(1);
                    info!(checkpoint_id = cp_id, "checkpoint timed out");
                }
                CheckpointResult::PipelineShutdown => {
                    return;
                }
            }
        }
    }

    /// Send barriers to all sources, then collect acks from all operators.
    async fn trigger_and_collect(&mut self, checkpoint_id: u64) -> CheckpointResult {
        let barrier = CheckpointBarrier::new(checkpoint_id, EventTimestamp::new(0));

        // Send barriers to all sources
        debug!(checkpoint_id, num_sources = self.barrier_senders.len(), "injecting barriers");
        for sender in &self.barrier_senders {
            if sender.send(barrier).await.is_err() {
                return CheckpointResult::PipelineShutdown;
            }
        }

        // Collect acks with timeout
        let mut received = HashSet::new();
        let deadline = tokio::time::Instant::now() + self.timeout;

        while received.len() < self.expected_operators.len() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return CheckpointResult::Timeout;
            }

            match tokio::time::timeout(remaining, self.ack_receiver.recv()).await {
                Ok(Some(ack)) if ack.checkpoint_id == checkpoint_id => {
                    debug!(checkpoint_id, operator = %ack.operator_name, "received ack");
                    // Persist operator state
                    if self
                        .store
                        .save_state(checkpoint_id, &ack.operator_name, &ack.state_bytes)
                        .await
                        .is_err()
                    {
                        return CheckpointResult::Timeout;
                    }
                    received.insert(ack.operator_name);
                }
                Ok(Some(_)) => {
                    // Ack for a different checkpoint — ignore (stale)
                }
                Ok(None) => {
                    return CheckpointResult::PipelineShutdown;
                }
                Err(_) => {
                    return CheckpointResult::Timeout;
                }
            }
        }

        // All acks received — commit
        if self.store.commit(checkpoint_id).await.is_ok() {
            CheckpointResult::Success
        } else {
            CheckpointResult::Timeout
        }
    }
}

enum CheckpointResult {
    Success,
    Timeout,
    PipelineShutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume_core::CheckpointAck;

    #[tokio::test]
    async fn test_successful_checkpoint_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path());

        let (barrier_tx, mut barrier_rx) = mpsc::channel::<CheckpointBarrier>(16);
        let (ack_tx, ack_rx) = mpsc::channel::<CheckpointAck>(16);

        let operators = vec!["op_a".to_string(), "op_b".to_string()];
        let mut coordinator = CheckpointCoordinator::new(
            vec![barrier_tx],
            ack_rx,
            operators,
            store,
            Duration::from_millis(50),
            Duration::from_secs(5),
            3,
        );

        // Simulate an operator that receives barriers and sends acks
        let ack_tx_clone = ack_tx.clone();
        let operator_task = tokio::spawn(async move {
            // Wait for the first barrier
            let barrier = barrier_rx.recv().await.unwrap();
            assert_eq!(barrier.checkpoint_id, 1);

            // Send acks from both operators
            ack_tx
                .send(CheckpointAck::new(
                    1,
                    "op_a".to_string(),
                    b"state_a".to_vec(),
                ))
                .await
                .unwrap();
            ack_tx_clone
                .send(CheckpointAck::new(
                    1,
                    "op_b".to_string(),
                    b"state_b".to_vec(),
                ))
                .await
                .unwrap();

            // Wait for second barrier, then drop the receiver to stop the coordinator
            let barrier2 = barrier_rx.recv().await.unwrap();
            assert_eq!(barrier2.checkpoint_id, 2);
            // Don't ack — drop everything to trigger shutdown
            drop(barrier_rx);
        });

        // Run coordinator — it will do checkpoint 1 successfully,
        // then checkpoint 2 will timeout or pipeline shuts down
        let cancel = CancellationToken::new();
        tokio::time::timeout(Duration::from_secs(5), coordinator.run(cancel))
            .await
            .ok();

        operator_task.await.unwrap();

        // Verify checkpoint 1 was committed
        let store = SnapshotStore::new(dir.path());
        assert_eq!(store.latest_checkpoint().await.unwrap(), Some(1));
        assert_eq!(store.load_state(1, "op_a").await.unwrap(), b"state_a");
        assert_eq!(store.load_state(1, "op_b").await.unwrap(), b"state_b");
    }

    #[tokio::test]
    async fn test_checkpoint_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path());

        let (barrier_tx, mut barrier_rx) = mpsc::channel::<CheckpointBarrier>(16);
        let (_ack_tx, ack_rx) = mpsc::channel::<CheckpointAck>(16);

        let operators = vec!["op_a".to_string()];
        let mut coordinator = CheckpointCoordinator::new(
            vec![barrier_tx],
            ack_rx,
            operators,
            store,
            Duration::from_millis(50),  // interval
            Duration::from_millis(100), // short timeout
            3,
        );

        // Consume barriers but never send acks — should timeout
        let drain_task = tokio::spawn(async move { while barrier_rx.recv().await.is_some() {} });

        // Run for a bit — coordinator should timeout and keep retrying
        let cancel = CancellationToken::new();
        tokio::time::timeout(Duration::from_millis(300), coordinator.run(cancel))
            .await
            .ok();

        drop(coordinator);
        drain_task.await.unwrap();

        // No checkpoint should be committed
        let store = SnapshotStore::new(dir.path());
        assert_eq!(store.latest_checkpoint().await.unwrap(), None);
    }
}
