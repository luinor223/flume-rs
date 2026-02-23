//! Kafka sink — produces messages to a Kafka topic.

use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use rdkafka::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord, Producer};

use flume_core::{FlumeError, FlumeResult, Record, Sink};

/// Produces records as messages to a Kafka topic.
///
/// Each record's value is converted to a string via `Display`.
/// If the record has a key, it is used as the Kafka message key.
pub struct KafkaSink<T: Display + Send + 'static> {
    producer: FutureProducer,
    topic: String,
    _marker: PhantomData<T>,
}

/// Builder for `KafkaSink`.
pub struct KafkaSinkBuilder {
    brokers: String,
    topic: String,
    extra_config: HashMap<String, String>,
}

impl KafkaSinkBuilder {
    pub fn new() -> Self {
        Self {
            brokers: String::new(),
            topic: String::new(),
            extra_config: HashMap::new(),
        }
    }

    pub fn brokers(mut self, brokers: impl Into<String>) -> Self {
        self.brokers = brokers.into();
        self
    }

    pub fn topic(mut self, topic: impl Into<String>) -> Self {
        self.topic = topic.into();
        self
    }

    pub fn config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_config.insert(key.into(), value.into());
        self
    }

    pub fn build<T: Display + Send + 'static>(self) -> FlumeResult<KafkaSink<T>> {
        if self.brokers.is_empty() {
            return Err(FlumeError::Config("brokers must be set".into()));
        }
        if self.topic.is_empty() {
            return Err(FlumeError::Config("topic must be set".into()));
        }

        let mut config = ClientConfig::new();
        config.set("bootstrap.servers", &self.brokers);

        for (k, v) in &self.extra_config {
            config.set(k, v);
        }

        let producer: FutureProducer =
            config.create().map_err(|e| FlumeError::Sink(Box::new(e)))?;

        Ok(KafkaSink {
            producer,
            topic: self.topic,
            _marker: PhantomData,
        })
    }
}

impl Default for KafkaSinkBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Display + Send + 'static> Sink<T> for KafkaSink<T> {
    fn write(
        &mut self,
        record: Record<T>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async move {
            let payload = record.value.to_string();
            let mut future_record = FutureRecord::to(&self.topic).payload(&payload);

            // Use record key as Kafka message key if present
            let key_bytes = record.key;
            if let Some(ref key) = key_bytes {
                future_record = future_record.key(key.as_slice());
            }

            self.producer
                .send(future_record, Duration::from_secs(5))
                .await
                .map_err(|(e, _)| FlumeError::Sink(Box::new(e)))?;

            Ok(())
        })
    }

    fn flush(&mut self) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async {
            self.producer.flush(Duration::from_secs(10)).unwrap();
            Ok(())
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_missing_brokers() {
        let result = KafkaSinkBuilder::new().topic("test").build::<String>();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_topic() {
        let result = KafkaSinkBuilder::new()
            .brokers("localhost:9092")
            .build::<String>();
        assert!(result.is_err());
    }
}
