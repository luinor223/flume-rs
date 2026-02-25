//! Network-aware collector that serializes records and sends them over gRPC.
//!
//! `NetworkCollector<T>` implements `Collector<T>` and batches records
//! into `DataBatch` proto messages. It respects credit-based flow control
//! by waiting for credits before sending.

use std::collections::HashMap;

use flume_core::{
    CheckpointBarrier, Collector, FlumeError, FlumeResult, Record, StreamElement, Watermark,
};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::trace;

use crate::proto::exchange::DataBatch;

/// A collector that serializes `StreamElement<T>` into bytes and sends
/// them as `DataBatch` proto messages to a gRPC outbound channel.
///
/// This is the "write side" of the network boundary. Records are serialized
/// via bitcode, batched, and sent when the batch is full or a barrier arrives.
pub struct NetworkCollector<T: Send + Serialize + 'static> {
    channel_id: String,
    batch_tx: mpsc::Sender<DataBatch>,
    /// Buffer of serialized elements waiting to be flushed.
    buffer: Vec<u8>,
    /// Number of records in the current buffer.
    record_count: u32,
    /// Maximum records per batch before auto-flush.
    batch_size: u32,
    /// Available credits (each credit allows sending one batch).
    credits: u32,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Send + Serialize + 'static> NetworkCollector<T> {
    pub fn new(channel_id: String, batch_tx: mpsc::Sender<DataBatch>, batch_size: u32) -> Self {
        Self {
            channel_id,
            batch_tx,
            buffer: Vec::new(),
            record_count: 0,
            batch_size,
            credits: 0,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Grant credits to this collector (called when CreditGrant messages arrive).
    pub fn grant_credits(&mut self, credits: u32) {
        self.credits = self.credits.saturating_add(credits);
    }

    /// Returns the number of available credits.
    pub fn available_credits(&self) -> u32 {
        self.credits
    }

    /// Flush the current buffer as a DataBatch.
    fn flush(&mut self) -> FlumeResult<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        if self.credits == 0 {
            return Err(FlumeError::ChannelClosed);
        }

        let batch = DataBatch {
            channel_id: self.channel_id.clone(),
            payload: std::mem::take(&mut self.buffer),
            record_count: self.record_count,
            is_barrier: false,
            checkpoint_id: 0,
        };

        self.record_count = 0;
        self.credits -= 1;

        trace!(
            channel_id = %self.channel_id,
            records = batch.record_count,
            bytes = batch.payload.len(),
            "flushing network batch"
        );

        self.batch_tx
            .try_send(batch)
            .map_err(|_| FlumeError::ChannelClosed)
    }

    fn serialize_element(&mut self, element: &StreamElement<T>) -> FlumeResult<()> {
        let bytes =
            bitcode::serialize(element).map_err(|e| FlumeError::Serialization(e.to_string()))?;
        // Length-prefix each serialized element for framing.
        let len = bytes.len() as u32;
        self.buffer.extend_from_slice(&len.to_le_bytes());
        self.buffer.extend_from_slice(&bytes);
        Ok(())
    }
}

impl<T: Send + Serialize + 'static> Collector<T> for NetworkCollector<T> {
    fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
        let element = StreamElement::Record(record);
        self.serialize_element(&element)?;
        self.record_count += 1;

        if self.record_count >= self.batch_size {
            self.flush()?;
        }
        Ok(())
    }

    fn collect_watermark(&mut self, watermark: Watermark) -> FlumeResult<()> {
        // Flush data before sending watermark.
        self.flush()?;

        let element = StreamElement::<T>::Watermark(watermark);
        let bytes =
            bitcode::serialize(&element).map_err(|e| FlumeError::Serialization(e.to_string()))?;

        let batch = DataBatch {
            channel_id: self.channel_id.clone(),
            payload: bytes,
            record_count: 0,
            is_barrier: false,
            checkpoint_id: 0,
        };

        if self.credits > 0 {
            self.credits -= 1;
            self.batch_tx
                .try_send(batch)
                .map_err(|_| FlumeError::ChannelClosed)
        } else {
            Err(FlumeError::ChannelClosed)
        }
    }

    fn collect_barrier(&mut self, barrier: CheckpointBarrier) -> FlumeResult<()> {
        // Flush data before sending barrier.
        self.flush()?;

        // Barriers are sent regardless of credits — they are control messages.
        let batch = DataBatch {
            channel_id: self.channel_id.clone(),
            payload: vec![],
            record_count: 0,
            is_barrier: true,
            checkpoint_id: barrier.checkpoint_id,
        };

        self.batch_tx
            .try_send(batch)
            .map_err(|_| FlumeError::ChannelClosed)
    }
}

/// Manages multiple `NetworkCollector`s for outbound channels from a single subtask.
pub struct NetworkCollectorManager<T: Send + Serialize + 'static> {
    collectors: HashMap<String, NetworkCollector<T>>,
}

impl<T: Send + Serialize + 'static> NetworkCollectorManager<T> {
    pub fn new() -> Self {
        Self {
            collectors: HashMap::new(),
        }
    }

    /// Add a collector for the given channel.
    pub fn add(&mut self, channel_id: String, collector: NetworkCollector<T>) {
        self.collectors.insert(channel_id, collector);
    }

    /// Get a mutable reference to a collector by channel_id.
    pub fn get_mut(&mut self, channel_id: &str) -> Option<&mut NetworkCollector<T>> {
        self.collectors.get_mut(channel_id)
    }

    /// Grant credits to a specific channel's collector.
    pub fn grant_credits(&mut self, channel_id: &str, credits: u32) {
        if let Some(collector) = self.collectors.get_mut(channel_id) {
            collector.grant_credits(credits);
        }
    }
}

impl<T: Send + Serialize + 'static> Default for NetworkCollectorManager<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use flume_core::EventTimestamp;

    use super::*;

    #[tokio::test]
    async fn test_network_collector_batch_and_flush() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut collector = NetworkCollector::<i32>::new("ch-1".into(), tx, 2);

        // Grant credits so we can send.
        collector.grant_credits(5);

        // First record: doesn't flush yet (batch_size=2).
        collector
            .collect(Record::new(42, EventTimestamp::new(100)))
            .unwrap();
        assert!(rx.try_recv().is_err()); // nothing sent yet

        // Second record: triggers flush.
        collector
            .collect(Record::new(43, EventTimestamp::new(200)))
            .unwrap();
        let batch = rx.recv().await.unwrap();
        assert_eq!(batch.channel_id, "ch-1");
        assert_eq!(batch.record_count, 2);
        assert!(!batch.is_barrier);
    }

    #[tokio::test]
    async fn test_network_collector_barrier_flushes_data() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut collector = NetworkCollector::<i32>::new("ch-1".into(), tx, 10);
        collector.grant_credits(5);

        // Buffer one record.
        collector
            .collect(Record::new(1, EventTimestamp::new(100)))
            .unwrap();

        // Barrier should flush data first, then send barrier batch.
        collector
            .collect_barrier(CheckpointBarrier::new(1, EventTimestamp::new(200)))
            .unwrap();

        let data_batch = rx.recv().await.unwrap();
        assert_eq!(data_batch.record_count, 1);
        assert!(!data_batch.is_barrier);

        let barrier_batch = rx.recv().await.unwrap();
        assert!(barrier_batch.is_barrier);
        assert_eq!(barrier_batch.checkpoint_id, 1);
    }

    #[tokio::test]
    async fn test_network_collector_no_credits() {
        let (tx, _rx) = mpsc::channel(16);
        let mut collector = NetworkCollector::<i32>::new("ch-1".into(), tx, 1);

        // No credits granted — collect fills buffer, flush should fail.
        let result = collector.collect(Record::new(1, EventTimestamp::new(100)));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_network_collector_credit_tracking() {
        let (tx, _rx) = mpsc::channel(16);
        let mut collector = NetworkCollector::<i32>::new("ch-1".into(), tx, 1);

        assert_eq!(collector.available_credits(), 0);
        collector.grant_credits(3);
        assert_eq!(collector.available_credits(), 3);

        collector
            .collect(Record::new(1, EventTimestamp::new(100)))
            .unwrap();
        assert_eq!(collector.available_credits(), 2);
    }

    #[tokio::test]
    async fn test_network_collector_manager() {
        let (tx1, _rx1) = mpsc::channel(16);
        let (tx2, _rx2) = mpsc::channel(16);

        let mut mgr = NetworkCollectorManager::<i32>::new();
        mgr.add("ch-1".into(), NetworkCollector::new("ch-1".into(), tx1, 10));
        mgr.add("ch-2".into(), NetworkCollector::new("ch-2".into(), tx2, 10));

        mgr.grant_credits("ch-1", 5);

        assert_eq!(mgr.get_mut("ch-1").unwrap().available_credits(), 5);
        assert_eq!(mgr.get_mut("ch-2").unwrap().available_credits(), 0);
        assert!(mgr.get_mut("ch-missing").is_none());
    }
}
