//! Built-in source implementations.

use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use flume_core::{EventTimestamp, FlumeResult, Record, Source, StreamElement};

/// Emits items from a `Vec<T>` one at a time. Each item becomes a Record
/// with the current wall-clock timestamp. Used for testing and examples.
pub struct InMemorySource<T> {
    items: std::vec::IntoIter<T>,
}

impl<T> InMemorySource<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items: items.into_iter(),
        }
    }
}

fn now_millis() -> EventTimestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    EventTimestamp::new(millis)
}

impl<T: Send + 'static> Source<T> for InMemorySource<T> {
    fn next(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<Option<StreamElement<T>>>> + Send + '_>> {
        Box::pin(async {
            match self.items.next() {
                Some(value) => Ok(Some(StreamElement::Record(Record::new(value, now_millis())))),
                None => Ok(None),
            }
        })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn restore(&mut self, _state: Vec<u8>) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}
