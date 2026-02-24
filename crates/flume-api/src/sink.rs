//! Built-in sink implementations.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use flume_core::{FlumeResult, Record, Sink};

/// Prints each record's value to stdout with a configurable prefix.
pub struct PrintSink {
    prefix: String,
}

impl PrintSink {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl<T: Debug + Send + 'static> Sink<T> for PrintSink {
    fn write(
        &mut self,
        record: Record<T>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        tracing::info!(prefix = %self.prefix, value = ?record.value, "sink output");
        Box::pin(async { Ok(()) })
    }

    fn flush(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn restore(
        &mut self,
        _state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// Collects all record values into a shared `Vec<T>` for assertions in tests.
pub struct CollectSink<T> {
    results: Arc<Mutex<Vec<T>>>,
}

impl<T> CollectSink<T> {
    /// Create a new sink and a handle to the results.
    pub fn new() -> (Self, Arc<Mutex<Vec<T>>>) {
        let results = Arc::new(Mutex::new(Vec::new()));
        let sink = Self {
            results: Arc::clone(&results),
        };
        (sink, results)
    }
}

impl<T: Send + 'static> Sink<T> for CollectSink<T> {
    fn write(
        &mut self,
        record: Record<T>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        self.results.lock().unwrap().push(record.value);
        Box::pin(async { Ok(()) })
    }

    fn flush(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn restore(
        &mut self,
        _state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}
