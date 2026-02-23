//! Kafka source — consumes messages from a Kafka topic as UTF-8 strings.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::{ClientConfig, TopicPartitionList};

use flume_core::{EventTimestamp, FlumeError, FlumeResult, Record, Source, StreamElement};

/// Consumes messages from a Kafka topic, emitting each as a `Record<String>`.
pub struct KafkaSource {
    consumer: StreamConsumer,
    topic: String,
}

/// Builder for `KafkaSource`.
pub struct KafkaSourceBuilder {
    brokers: String,
    topic: String,
    group_id: String,
    extra_config: HashMap<String, String>,
}

impl KafkaSourceBuilder {
    pub fn new() -> Self {
        Self {
            brokers: String::new(),
            topic: String::new(),
            group_id: String::new(),
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

    pub fn group_id(mut self, group_id: impl Into<String>) -> Self {
        self.group_id = group_id.into();
        self
    }

    pub fn config(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_config.insert(key.into(), value.into());
        self
    }

    pub fn build(self) -> FlumeResult<KafkaSource> {
        if self.brokers.is_empty() {
            return Err(FlumeError::Config("brokers must be set".into()));
        }
        if self.topic.is_empty() {
            return Err(FlumeError::Config("topic must be set".into()));
        }
        if self.group_id.is_empty() {
            return Err(FlumeError::Config("group_id must be set".into()));
        }

        let mut config = ClientConfig::new();
        config
            .set("bootstrap.servers", &self.brokers)
            .set("group.id", &self.group_id)
            .set("auto.offset.reset", "earliest");

        for (k, v) in &self.extra_config {
            config.set(k, v);
        }

        let consumer: StreamConsumer = config
            .create()
            .map_err(|e| FlumeError::Source(Box::new(e)))?;

        consumer
            .subscribe(&[&self.topic])
            .map_err(|e| FlumeError::Source(Box::new(e)))?;

        Ok(KafkaSource {
            consumer,
            topic: self.topic,
        })
    }
}

impl Default for KafkaSourceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> EventTimestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    EventTimestamp::new(millis)
}

impl Source<String> for KafkaSource {
    fn next(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<Option<StreamElement<String>>>> + Send + '_>> {
        Box::pin(async {
            let msg = self
                .consumer
                .recv()
                .await
                .map_err(|e| FlumeError::Source(Box::new(e)))?;

            let payload = match msg.payload_view::<str>() {
                Some(Ok(text)) => text.to_owned(),
                Some(Err(e)) => return Err(FlumeError::Source(Box::new(e))),
                None => {
                    return Ok(Some(StreamElement::Record(Record::new(
                        String::new(),
                        now_millis(),
                    ))));
                }
            };

            let timestamp = msg
                .timestamp()
                .to_millis()
                .map(|ms| EventTimestamp::new(ms as u64))
                .unwrap_or_else(now_millis);

            let mut record = Record::new(payload, timestamp);
            if let Some(key_bytes) = msg.key() {
                record = record.with_key(key_bytes.to_vec());
            }

            Ok(Some(StreamElement::Record(record)))
        })
    }

    fn snapshot(&self) -> Pin<Box<dyn Future<Output = FlumeResult<Vec<u8>>> + Send + '_>> {
        Box::pin(async {
            let assignment = self
                .consumer
                .assignment()
                .map_err(|e| FlumeError::Source(Box::new(e)))?;

            let positions = self
                .consumer
                .position()
                .map_err(|e| FlumeError::Source(Box::new(e)))?;

            // Serialize as: topic_len(u32) + topic + partition_count(u32) + [partition(i32) + offset(i64)]...
            let mut buf = Vec::new();
            let topic_bytes = self.topic.as_bytes();
            buf.extend_from_slice(&(topic_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(topic_bytes);

            let elements = positions.elements();
            let mut partition_data = Vec::new();
            for elem in &elements {
                if let rdkafka::Offset::Offset(off) = elem.offset() {
                    partition_data.push((elem.partition(), off));
                }
            }
            // Also include assigned partitions with no position yet
            for elem in &assignment.elements() {
                let partition = elem.partition();
                if !partition_data.iter().any(|(p, _)| *p == partition) {
                    partition_data.push((partition, 0));
                }
            }

            buf.extend_from_slice(&(partition_data.len() as u32).to_le_bytes());
            for (partition, offset) in partition_data {
                buf.extend_from_slice(&partition.to_le_bytes());
                buf.extend_from_slice(&offset.to_le_bytes());
            }

            Ok(buf)
        })
    }

    fn restore(
        &mut self,
        state: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = FlumeResult<()>> + Send + '_>> {
        Box::pin(async move {
            if state.is_empty() {
                return Ok(());
            }

            let mut cursor = &state[..];

            // Read topic
            if cursor.len() < 4 {
                return Err(FlumeError::Checkpoint("invalid state: too short".into()));
            }
            let topic_len = u32::from_le_bytes(cursor[..4].try_into().unwrap()) as usize;
            cursor = &cursor[4..];

            if cursor.len() < topic_len {
                return Err(FlumeError::Checkpoint(
                    "invalid state: topic truncated".into(),
                ));
            }
            let _topic = std::str::from_utf8(&cursor[..topic_len])
                .map_err(|_| FlumeError::Checkpoint("invalid topic utf8".into()))?;
            cursor = &cursor[topic_len..];

            // Read partition count
            if cursor.len() < 4 {
                return Err(FlumeError::Checkpoint(
                    "invalid state: no partition count".into(),
                ));
            }
            let count = u32::from_le_bytes(cursor[..4].try_into().unwrap()) as usize;
            cursor = &cursor[4..];

            let mut tpl = TopicPartitionList::new();
            for _ in 0..count {
                if cursor.len() < 12 {
                    return Err(FlumeError::Checkpoint(
                        "invalid state: partition data truncated".into(),
                    ));
                }
                let partition = i32::from_le_bytes(cursor[..4].try_into().unwrap());
                let offset = i64::from_le_bytes(cursor[4..12].try_into().unwrap());
                cursor = &cursor[12..];
                tpl.add_partition_offset(&self.topic, partition, rdkafka::Offset::Offset(offset))
                    .map_err(|e| FlumeError::Checkpoint(format!("failed to set offset: {e}")))?;
            }

            self.consumer
                .assign(&tpl)
                .map_err(|e| FlumeError::Source(Box::new(e)))?;

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_missing_brokers() {
        let result = KafkaSourceBuilder::new()
            .topic("test")
            .group_id("grp")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_topic() {
        let result = KafkaSourceBuilder::new()
            .brokers("localhost:9092")
            .group_id("grp")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_group_id() {
        let result = KafkaSourceBuilder::new()
            .brokers("localhost:9092")
            .topic("test")
            .build();
        assert!(result.is_err());
    }
}
