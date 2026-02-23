//! Barrier aligner for Chandy-Lamport checkpoint alignment.
//!
//! For multi-input operators, buffers records on fast inputs until
//! checkpoint barriers arrive on all inputs before emitting the barrier.
//! Single-input operators use a fast path with no buffering.

use std::collections::VecDeque;

use flume_core::{CheckpointBarrier, StreamElement};
use tokio::sync::mpsc;

/// Output produced by the barrier aligner.
#[derive(Debug)]
pub enum AlignerOutput<T> {
    /// A regular stream element to process.
    Element(StreamElement<T>),
    /// All inputs have received this barrier — time to snapshot state.
    BarrierAligned(CheckpointBarrier),
}

/// Aligns checkpoint barriers across multiple input channels.
///
/// For single-input operators, barriers pass through directly.
/// For multi-input operators, when a barrier arrives on one input:
/// 1. That input is "blocked" — its subsequent records are buffered
/// 2. Other inputs continue producing records normally
/// 3. When all inputs have received the barrier, it is emitted
/// 4. Buffered records from blocked inputs are drained
pub struct BarrierAligner<T: Send + 'static> {
    inputs: Vec<mpsc::Receiver<StreamElement<T>>>,
    /// Which inputs have received the current barrier.
    barrier_received: Vec<bool>,
    /// Buffered records from blocked (barrier-received) inputs.
    blocked_buffers: Vec<VecDeque<StreamElement<T>>>,
    /// The barrier currently being aligned (set when first input receives it).
    pending_barrier: Option<CheckpointBarrier>,
    /// Drain queue: after alignment, drain buffered records before next poll.
    drain_index: usize,
}

impl<T: Send + 'static> BarrierAligner<T> {
    pub fn new(inputs: Vec<mpsc::Receiver<StreamElement<T>>>) -> Self {
        let n = inputs.len();
        Self {
            inputs,
            barrier_received: vec![false; n],
            blocked_buffers: (0..n).map(|_| VecDeque::new()).collect(),
            pending_barrier: None,
            drain_index: 0,
        }
    }

    /// Number of input channels.
    pub fn num_inputs(&self) -> usize {
        self.inputs.len()
    }

    /// Produce the next output element.
    ///
    /// Returns `None` only when all input channels are closed.
    pub async fn next(&mut self) -> Option<AlignerOutput<T>> {
        // Phase 1: drain any buffered records from a completed alignment
        while self.drain_index < self.blocked_buffers.len() {
            if let Some(elem) = self.blocked_buffers[self.drain_index].pop_front() {
                return Some(AlignerOutput::Element(elem));
            }
            self.drain_index += 1;
        }

        // Phase 2: poll inputs
        if self.inputs.len() == 1 {
            return self.next_single().await;
        }
        self.next_multi().await
    }

    /// Fast path for single-input: no buffering needed.
    async fn next_single(&mut self) -> Option<AlignerOutput<T>> {
        let elem = self.inputs[0].recv().await?;
        match elem {
            StreamElement::CheckpointBarrier(b) => Some(AlignerOutput::BarrierAligned(b)),
            other => Some(AlignerOutput::Element(other)),
        }
    }

    /// Multi-input alignment using Chandy-Lamport.
    async fn next_multi(&mut self) -> Option<AlignerOutput<T>> {
        loop {
            // Try to receive from any non-closed input.
            // We use a select-like approach: try each unblocked input, then blocked ones.
            let (idx, elem) = self.recv_any().await?;

            match elem {
                StreamElement::CheckpointBarrier(barrier) => {
                    self.barrier_received[idx] = true;
                    if self.pending_barrier.is_none() {
                        self.pending_barrier = Some(barrier);
                    }

                    // Check if all inputs have received the barrier
                    if self.barrier_received.iter().all(|&b| b) {
                        let barrier = self.pending_barrier.take().unwrap();
                        // Reset alignment state
                        self.barrier_received.fill(false);
                        self.drain_index = 0;
                        return Some(AlignerOutput::BarrierAligned(barrier));
                    }
                }
                other => {
                    if self.barrier_received[idx] {
                        // This input is blocked — buffer the record
                        self.blocked_buffers[idx].push_back(other);
                    } else {
                        return Some(AlignerOutput::Element(other));
                    }
                }
            }
        }
    }

    /// Receive from any input channel that hasn't been closed.
    /// Prefers unblocked inputs (those that haven't received a barrier yet).
    async fn recv_any(&mut self) -> Option<(usize, StreamElement<T>)> {
        // Build a list of indices to poll — unblocked first, then blocked
        // We use tokio::select! on all active channels.
        loop {
            // We need to poll all inputs fairly. Use a manual select approach.
            // tokio::select! with a dynamic number of branches requires a macro workaround.
            // Instead, we use a polling loop with try_recv for fairness.

            // First pass: try_recv from unblocked inputs (non-blocking)
            for i in 0..self.inputs.len() {
                if !self.barrier_received[i] {
                    match self.inputs[i].try_recv() {
                        Ok(elem) => return Some((i, elem)),
                        Err(mpsc::error::TryRecvError::Disconnected) => continue,
                        Err(mpsc::error::TryRecvError::Empty) => continue,
                    }
                }
            }

            // Second pass: try_recv from blocked inputs (they might have barriers)
            for i in 0..self.inputs.len() {
                if self.barrier_received[i] {
                    match self.inputs[i].try_recv() {
                        Ok(elem) => return Some((i, elem)),
                        Err(mpsc::error::TryRecvError::Disconnected) => continue,
                        Err(mpsc::error::TryRecvError::Empty) => continue,
                    }
                }
            }

            // Nothing available — do a blocking recv on any open channel.
            // Find the first open channel and block on it, yielding to let others send.
            tokio::task::yield_now().await;

            // Check if all channels are disconnected
            let mut any_open = false;
            for i in 0..self.inputs.len() {
                // Peek by trying recv
                match self.inputs[i].try_recv() {
                    Ok(elem) => return Some((i, elem)),
                    Err(mpsc::error::TryRecvError::Empty) => {
                        any_open = true;
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => continue,
                }
            }

            if !any_open {
                return None;
            }

            // Yield again before retrying
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume_core::{EventTimestamp, Record, Watermark};

    #[tokio::test]
    async fn test_single_input_passthrough() {
        let (tx, rx) = mpsc::channel(16);
        let mut aligner = BarrierAligner::new(vec![rx]);

        // Send a record
        tx.send(StreamElement::Record(Record::new(42, EventTimestamp::new(100))))
            .await
            .unwrap();
        let out = aligner.next().await.unwrap();
        assert!(matches!(out, AlignerOutput::Element(StreamElement::Record(_))));

        // Send a watermark
        tx.send(StreamElement::Watermark(Watermark::new(EventTimestamp::new(200))))
            .await
            .unwrap();
        let out = aligner.next().await.unwrap();
        assert!(matches!(
            out,
            AlignerOutput::Element(StreamElement::Watermark(_))
        ));

        // Send a barrier — should become BarrierAligned
        let barrier = CheckpointBarrier::new(1, EventTimestamp::new(300));
        tx.send(StreamElement::CheckpointBarrier(barrier))
            .await
            .unwrap();
        let out = aligner.next().await.unwrap();
        match out {
            AlignerOutput::BarrierAligned(b) => assert_eq!(b.checkpoint_id, 1),
            _ => panic!("expected BarrierAligned"),
        }

        // Close channel
        drop(tx);
        assert!(aligner.next().await.is_none());
    }

    #[tokio::test]
    async fn test_multi_input_alignment() {
        let (tx0, rx0) = mpsc::channel(16);
        let (tx1, rx1) = mpsc::channel(16);
        let mut aligner = BarrierAligner::new(vec![rx0, rx1]);

        let barrier = CheckpointBarrier::new(1, EventTimestamp::new(500));

        // Input 0 sends: record, barrier
        tx0.send(StreamElement::Record(Record::new(10, EventTimestamp::new(100))))
            .await
            .unwrap();
        tx0.send(StreamElement::CheckpointBarrier(barrier))
            .await
            .unwrap();
        // After barrier on input 0, records on input 0 should be buffered
        tx0.send(StreamElement::Record(Record::new(
            11,
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();

        // Input 1 sends: record, record, barrier
        tx1.send(StreamElement::Record(Record::new(20, EventTimestamp::new(100))))
            .await
            .unwrap();
        tx1.send(StreamElement::Record(Record::new(
            21,
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();
        tx1.send(StreamElement::CheckpointBarrier(barrier))
            .await
            .unwrap();

        // Collect outputs until we see the aligned barrier
        let mut records = Vec::new();
        let mut barrier_seen = false;
        let mut post_barrier_records = Vec::new();

        loop {
            let out = aligner.next().await.unwrap();
            match out {
                AlignerOutput::Element(StreamElement::Record(r)) => {
                    if barrier_seen {
                        post_barrier_records.push(r.value);
                    } else {
                        records.push(r.value);
                    }
                }
                AlignerOutput::BarrierAligned(b) => {
                    assert_eq!(b.checkpoint_id, 1);
                    barrier_seen = true;
                }
                _ => {}
            }
            // After barrier, drain the buffered record from input 0
            if barrier_seen && !post_barrier_records.is_empty() {
                break;
            }
        }

        // Pre-barrier records: 10 from input 0, then 20 and 21 from input 1
        assert!(records.contains(&10));
        assert!(records.contains(&20));
        assert!(records.contains(&21));

        // Post-barrier buffered record: 11 from input 0
        assert_eq!(post_barrier_records, vec![11]);
    }

    #[tokio::test]
    async fn test_multi_input_barrier_ordering() {
        // Test that barriers from different checkpoints are handled sequentially
        let (tx0, rx0) = mpsc::channel::<StreamElement<i32>>(16);
        let (tx1, rx1) = mpsc::channel::<StreamElement<i32>>(16);
        let mut aligner = BarrierAligner::new(vec![rx0, rx1]);

        let b1 = CheckpointBarrier::new(1, EventTimestamp::new(100));
        let b2 = CheckpointBarrier::new(2, EventTimestamp::new(200));

        // First checkpoint
        tx0.send(StreamElement::CheckpointBarrier(b1)).await.unwrap();
        tx1.send(StreamElement::CheckpointBarrier(b1)).await.unwrap();

        // Second checkpoint
        tx0.send(StreamElement::CheckpointBarrier(b2)).await.unwrap();
        tx1.send(StreamElement::CheckpointBarrier(b2)).await.unwrap();

        // Should see barrier 1 first, then barrier 2
        let out1 = aligner.next().await.unwrap();
        match out1 {
            AlignerOutput::BarrierAligned(b) => assert_eq!(b.checkpoint_id, 1),
            _ => panic!("expected barrier 1"),
        }

        let out2 = aligner.next().await.unwrap();
        match out2 {
            AlignerOutput::BarrierAligned(b) => assert_eq!(b.checkpoint_id, 2),
            _ => panic!("expected barrier 2"),
        }
    }
}
