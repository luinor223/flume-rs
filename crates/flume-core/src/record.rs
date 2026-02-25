//! Core data types that flow through the processing pipeline.

use serde::{Deserialize, Serialize};

use crate::EventTimestamp;

/// A single data record flowing through the pipeline.
///
/// Generic over `T` to enable monomorphization — the compiler generates
/// specialized code for each concrete type, eliminating virtual dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record<T> {
    /// The payload.
    pub value: T,
    /// Event-time timestamp (when the event occurred in the real world).
    pub timestamp: EventTimestamp,
    /// Partition key (serialized bytes). `None` before `key_by()`.
    pub key: Option<Vec<u8>>,
}

impl<T> Record<T> {
    /// Create a record with no partition key.
    pub fn new(value: T, timestamp: EventTimestamp) -> Self {
        Self {
            value,
            timestamp,
            key: None,
        }
    }

    /// Attach a partition key.
    pub fn with_key(mut self, key: Vec<u8>) -> Self {
        self.key = Some(key);
        self
    }

    /// Transform the value, preserving timestamp and key.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Record<U> {
        Record {
            value: f(self.value),
            timestamp: self.timestamp,
            key: self.key,
        }
    }
}

/// Declares that no more records with event-time <= this timestamp
/// will arrive on this channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watermark {
    pub timestamp: EventTimestamp,
}

impl Watermark {
    pub fn new(timestamp: EventTimestamp) -> Self {
        Self { timestamp }
    }
}

/// Injected by the CheckpointCoordinator to trigger state snapshots.
/// Flows inline with data records through the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointBarrier {
    pub checkpoint_id: u64,
    pub timestamp: EventTimestamp,
}

impl CheckpointBarrier {
    pub fn new(checkpoint_id: u64, timestamp: EventTimestamp) -> Self {
        Self {
            checkpoint_id,
            timestamp,
        }
    }
}

/// Acknowledgment sent by an operator after snapshotting its state
/// in response to a `CheckpointBarrier`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointAck {
    pub checkpoint_id: u64,
    pub operator_name: String,
    pub state_bytes: Vec<u8>,
}

impl CheckpointAck {
    pub fn new(checkpoint_id: u64, operator_name: String, state_bytes: Vec<u8>) -> Self {
        Self {
            checkpoint_id,
            operator_name,
            state_bytes,
        }
    }
}

/// The unit of transport between operators. Wraps data and control messages
/// in a single enum so they flow inline through the same channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamElement<T> {
    Record(Record<T>),
    Watermark(Watermark),
    CheckpointBarrier(CheckpointBarrier),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_new() {
        let r = Record::new(42, EventTimestamp::new(1000));
        assert_eq!(r.value, 42);
        assert_eq!(r.timestamp, EventTimestamp::new(1000));
        assert!(r.key.is_none());
    }

    #[test]
    fn test_record_with_key() {
        let r = Record::new(42, EventTimestamp::new(1000)).with_key(vec![1, 2, 3]);
        assert_eq!(r.key, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_record_map() {
        let r = Record::new(10, EventTimestamp::new(500)).with_key(vec![1]);
        let mapped = r.map(|v| v * 2);
        assert_eq!(mapped.value, 20);
        assert_eq!(mapped.timestamp, EventTimestamp::new(500));
        assert_eq!(mapped.key, Some(vec![1]));
    }

    #[test]
    fn test_serde_roundtrip_record() {
        let r = Record::new(42i32, EventTimestamp::new(1000)).with_key(vec![1, 2]);
        let bytes = bincode::serialize(&r).unwrap();
        let r2: Record<i32> = bincode::deserialize(&bytes).unwrap();
        assert_eq!(r2.value, 42);
        assert_eq!(r2.timestamp, EventTimestamp::new(1000));
        assert_eq!(r2.key, Some(vec![1, 2]));
    }

    #[test]
    fn test_serde_roundtrip_stream_element() {
        let elem =
            StreamElement::Record(Record::new("hello".to_string(), EventTimestamp::new(500)));
        let bytes = bincode::serialize(&elem).unwrap();
        let elem2: StreamElement<String> = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(elem2, StreamElement::Record(r) if r.value == "hello"));

        let wm = StreamElement::<i32>::Watermark(Watermark::new(EventTimestamp::new(200)));
        let bytes = bincode::serialize(&wm).unwrap();
        let wm2: StreamElement<i32> = bincode::deserialize(&bytes).unwrap();
        assert!(
            matches!(wm2, StreamElement::Watermark(w) if w.timestamp == EventTimestamp::new(200))
        );

        let barrier = StreamElement::<i32>::CheckpointBarrier(CheckpointBarrier::new(
            7,
            EventTimestamp::new(300),
        ));
        let bytes = bincode::serialize(&barrier).unwrap();
        let barrier2: StreamElement<i32> = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(barrier2, StreamElement::CheckpointBarrier(b) if b.checkpoint_id == 7));
    }

    #[test]
    fn test_serde_roundtrip_checkpoint_ack() {
        let ack = CheckpointAck::new(5, "op-1".to_string(), vec![10, 20, 30]);
        let bytes = bincode::serialize(&ack).unwrap();
        let ack2: CheckpointAck = bincode::deserialize(&bytes).unwrap();
        assert_eq!(ack2.checkpoint_id, 5);
        assert_eq!(ack2.operator_name, "op-1");
        assert_eq!(ack2.state_bytes, vec![10, 20, 30]);
    }

    #[test]
    fn test_stream_element_variants() {
        let data: StreamElement<i32> =
            StreamElement::Record(Record::new(1, EventTimestamp::new(100)));
        assert!(matches!(data, StreamElement::Record(_)));

        let wm = StreamElement::<i32>::Watermark(Watermark::new(EventTimestamp::new(200)));
        assert!(matches!(wm, StreamElement::Watermark(_)));

        let barrier = StreamElement::<i32>::CheckpointBarrier(CheckpointBarrier::new(
            1,
            EventTimestamp::new(300),
        ));
        assert!(matches!(barrier, StreamElement::CheckpointBarrier(_)));
    }
}
