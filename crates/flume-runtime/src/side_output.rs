//! Side output channel infrastructure.
//!
//! `SideOutputCollectors` implements `SideOutputEmitter` (from `flume-core`)
//! using tokio mpsc channels. Process operators register typed channels, and
//! side output values are type-erased, downcast, and forwarded through them.

use std::any::Any;
use std::collections::HashMap;

use flume_core::{
    EventTimestamp, FlumeError, FlumeResult, Record, SideOutputEmitter, StreamElement,
};
use tokio::sync::mpsc;

/// A type-erased sender closure: takes a boxed `Any + Send`, a timestamp, and
/// an optional key, downcasts the value, and sends it through a typed channel.
type ErasedSender =
    Box<dyn FnMut(Box<dyn Any + Send>, EventTimestamp, Option<Vec<u8>>) -> FlumeResult<()> + Send>;

/// Holds side output channels keyed by tag ID.
///
/// Each channel is registered with a concrete type via [`register`](Self::register).
/// The `SideOutputEmitter` implementation downcasts the erased value at runtime.
pub struct SideOutputCollectors {
    senders: HashMap<String, ErasedSender>,
}

impl SideOutputCollectors {
    /// Create an empty set of side output collectors.
    pub fn new() -> Self {
        Self {
            senders: HashMap::new(),
        }
    }

    /// Register a typed side output channel for the given tag.
    ///
    /// Values emitted to this tag will be downcast to `T` and forwarded as
    /// `StreamElement::Record` through the provided sender.
    pub fn register<T: Send + 'static>(
        &mut self,
        tag_id: String,
        sender: mpsc::Sender<StreamElement<T>>,
    ) {
        let erased: ErasedSender = Box::new(move |value, timestamp, key| {
            let typed = value.downcast::<T>().map_err(|_| {
                FlumeError::Operator("side output type mismatch during downcast".into())
            })?;
            let mut record = Record::new(*typed, timestamp);
            record.key = key;
            sender
                .try_send(StreamElement::Record(record))
                .map_err(|_| FlumeError::ChannelClosed)
        });
        self.senders.insert(tag_id, erased);
    }
}

impl Default for SideOutputCollectors {
    fn default() -> Self {
        Self::new()
    }
}

impl SideOutputEmitter for SideOutputCollectors {
    fn emit(
        &mut self,
        tag_id: &str,
        value: Box<dyn Any + Send>,
        timestamp: EventTimestamp,
        key: Option<Vec<u8>>,
    ) -> FlumeResult<()> {
        let sender = self
            .senders
            .get_mut(tag_id)
            .ok_or_else(|| FlumeError::Operator(format!("unknown side output tag: {tag_id}")))?;
        sender(value, timestamp, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_emit() {
        let (tx, mut rx) = mpsc::channel::<StreamElement<String>>(16);

        let mut collectors = SideOutputCollectors::new();
        collectors.register("rejected".to_string(), tx);

        collectors
            .emit(
                "rejected",
                Box::new("bad-record".to_string()),
                EventTimestamp::new(100),
                None,
            )
            .unwrap();

        let element = rx.recv().await.unwrap();
        if let StreamElement::Record(record) = element {
            assert_eq!(record.value, "bad-record");
            assert_eq!(record.timestamp, EventTimestamp::new(100));
            assert!(record.key.is_none());
        } else {
            panic!("expected Record");
        }
    }

    #[tokio::test]
    async fn test_emit_with_key() {
        let (tx, mut rx) = mpsc::channel::<StreamElement<i32>>(16);

        let mut collectors = SideOutputCollectors::new();
        collectors.register("overflow".to_string(), tx);

        collectors
            .emit(
                "overflow",
                Box::new(42i32),
                EventTimestamp::new(200),
                Some(vec![1, 2, 3]),
            )
            .unwrap();

        let element = rx.recv().await.unwrap();
        if let StreamElement::Record(record) = element {
            assert_eq!(record.value, 42);
            assert_eq!(record.key, Some(vec![1, 2, 3]));
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn test_emit_unknown_tag_returns_error() {
        let mut collectors = SideOutputCollectors::new();
        let result = collectors.emit(
            "nonexistent",
            Box::new(42i32),
            EventTimestamp::new(100),
            None,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_emit_closed_channel_returns_error() {
        let (tx, rx) = mpsc::channel::<StreamElement<i32>>(16);
        drop(rx);

        let mut collectors = SideOutputCollectors::new();
        collectors.register("tag".to_string(), tx);

        let result = collectors.emit("tag", Box::new(1i32), EventTimestamp::new(100), None);
        assert!(result.is_err());
    }
}
