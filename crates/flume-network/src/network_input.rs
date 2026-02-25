//! Network input that deserializes batches received from gRPC into `StreamElement<T>`.
//!
//! This is the "read side" of the network boundary. It consumes `DataBatch`
//! proto messages, deserializes the payload using bitcode, and yields
//! `StreamElement<T>` values.

use std::collections::VecDeque;

use flume_core::{CheckpointBarrier, EventTimestamp, StreamElement};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

use crate::proto::exchange::DataBatch;

/// Receives `DataBatch` proto messages and deserializes them into
/// `StreamElement<T>` values that can be fed into local operator inputs.
pub struct NetworkInput<T> {
    batch_rx: mpsc::Receiver<DataBatch>,
    /// Buffered elements from the current batch being consumed.
    element_buffer: VecDeque<StreamElement<T>>,
}

impl<T: DeserializeOwned + Send + 'static> NetworkInput<T> {
    pub fn new(batch_rx: mpsc::Receiver<DataBatch>) -> Self {
        Self {
            batch_rx,
            element_buffer: VecDeque::new(),
        }
    }

    /// Receive the next stream element, or `None` if the channel is closed.
    pub async fn recv(&mut self) -> Option<StreamElement<T>> {
        loop {
            // Return buffered elements first.
            if let Some(elem) = self.element_buffer.pop_front() {
                return Some(elem);
            }

            // Buffer exhausted, get next batch.
            let batch = self.batch_rx.recv().await?;

            // Handle barrier batches.
            if batch.is_barrier {
                return Some(StreamElement::CheckpointBarrier(CheckpointBarrier::new(
                    batch.checkpoint_id,
                    EventTimestamp::new(0),
                )));
            }

            // Deserialize payload into stream elements.
            let mut offset = 0;
            let payload = &batch.payload;

            while offset + 4 <= payload.len() {
                // Read length prefix.
                let len = u32::from_le_bytes([
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ]) as usize;
                offset += 4;

                if offset + len > payload.len() {
                    break;
                }

                match bitcode::deserialize::<StreamElement<T>>(&payload[offset..offset + len]) {
                    Ok(elem) => self.element_buffer.push_back(elem),
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to deserialize network element");
                    }
                }
                offset += len;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use flume_core::{EventTimestamp, Record, Watermark};
    use serde::Serialize;

    use super::*;

    fn serialize_elements<T: Serialize>(elements: &[StreamElement<T>]) -> Vec<u8> {
        let mut buf = Vec::new();
        for elem in elements {
            let bytes = bitcode::serialize(elem).unwrap();
            let len = bytes.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&bytes);
        }
        buf
    }

    #[tokio::test]
    async fn test_network_input_deserialize_records() {
        let (tx, rx) = mpsc::channel(16);
        let mut input = NetworkInput::<i32>::new(rx);

        let elements = vec![
            StreamElement::Record(Record::new(42, EventTimestamp::new(100))),
            StreamElement::Record(Record::new(43, EventTimestamp::new(200))),
        ];
        let payload = serialize_elements(&elements);

        tx.send(DataBatch {
            channel_id: "ch-1".into(),
            payload,
            record_count: 2,
            is_barrier: false,
            checkpoint_id: 0,
        })
        .await
        .unwrap();

        let elem1 = input.recv().await.unwrap();
        assert!(matches!(elem1, StreamElement::Record(r) if r.value == 42));

        let elem2 = input.recv().await.unwrap();
        assert!(matches!(elem2, StreamElement::Record(r) if r.value == 43));
    }

    #[tokio::test]
    async fn test_network_input_barrier() {
        let (tx, rx) = mpsc::channel(16);
        let mut input = NetworkInput::<i32>::new(rx);

        tx.send(DataBatch {
            channel_id: "ch-1".into(),
            payload: vec![],
            record_count: 0,
            is_barrier: true,
            checkpoint_id: 7,
        })
        .await
        .unwrap();

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::CheckpointBarrier(b) if b.checkpoint_id == 7));
    }

    #[tokio::test]
    async fn test_network_input_watermark() {
        let (tx, rx) = mpsc::channel(16);
        let mut input = NetworkInput::<i32>::new(rx);

        let elements = vec![StreamElement::<i32>::Watermark(Watermark::new(
            EventTimestamp::new(500),
        ))];
        let payload = serialize_elements(&elements);

        tx.send(DataBatch {
            channel_id: "ch-1".into(),
            payload,
            record_count: 0,
            is_barrier: false,
            checkpoint_id: 0,
        })
        .await
        .unwrap();

        let elem = input.recv().await.unwrap();
        assert!(
            matches!(elem, StreamElement::Watermark(w) if w.timestamp == EventTimestamp::new(500))
        );
    }

    #[tokio::test]
    async fn test_network_input_closed_channel() {
        let (tx, rx) = mpsc::channel::<DataBatch>(16);
        let mut input = NetworkInput::<i32>::new(rx);

        drop(tx);
        let result = input.recv().await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_network_input_multi_batch() {
        let (tx, rx) = mpsc::channel(16);
        let mut input = NetworkInput::<i32>::new(rx);

        // Send two separate batches.
        let batch1 = serialize_elements(&[StreamElement::Record(Record::new(
            1,
            EventTimestamp::new(100),
        ))]);
        let batch2 = serialize_elements(&[StreamElement::Record(Record::new(
            2,
            EventTimestamp::new(200),
        ))]);

        tx.send(DataBatch {
            channel_id: "ch-1".into(),
            payload: batch1,
            record_count: 1,
            is_barrier: false,
            checkpoint_id: 0,
        })
        .await
        .unwrap();

        tx.send(DataBatch {
            channel_id: "ch-1".into(),
            payload: batch2,
            record_count: 1,
            is_barrier: false,
            checkpoint_id: 0,
        })
        .await
        .unwrap();

        let elem1 = input.recv().await.unwrap();
        assert!(matches!(elem1, StreamElement::Record(r) if r.value == 1));

        let elem2 = input.recv().await.unwrap();
        assert!(matches!(elem2, StreamElement::Record(r) if r.value == 2));
    }
}
