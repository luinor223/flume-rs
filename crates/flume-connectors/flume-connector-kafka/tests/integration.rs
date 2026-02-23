//! Integration tests for Kafka connector using testcontainers.
//!
//! These tests require Docker. Run with: `cargo test -p flume-connector-kafka -- --ignored`

use std::time::Duration;

use flume_connector_kafka::{KafkaSinkBuilder, KafkaSourceBuilder};
use flume_core::{EventTimestamp, Record, Sink, Source, StreamElement};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::kafka::apache::Kafka;

async fn start_kafka() -> (testcontainers::ContainerAsync<Kafka>, String) {
    let container = Kafka::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(9093).await.unwrap();
    let brokers = format!("127.0.0.1:{port}");
    // Give Kafka a moment to be fully ready
    tokio::time::sleep(Duration::from_secs(2)).await;
    (container, brokers)
}

#[tokio::test]
#[ignore]
async fn test_produce_and_consume() {
    let (_container, brokers) = start_kafka().await;
    let topic = "test-produce-consume";

    // Produce messages
    {
        let mut sink: flume_connector_kafka::KafkaSink<String> = KafkaSinkBuilder::new()
            .brokers(&brokers)
            .topic(topic)
            .build()
            .unwrap();

        for i in 0..5 {
            let record = Record::new(format!("message-{i}"), EventTimestamp::new(i as u64));
            sink.write(record).await.unwrap();
        }
        sink.flush().await.unwrap();
    }

    // Consume messages
    let mut source = KafkaSourceBuilder::new()
        .brokers(&brokers)
        .topic(topic)
        .group_id("test-group")
        .config("auto.offset.reset", "earliest")
        .build()
        .unwrap();

    let mut received = Vec::new();
    for _ in 0..5 {
        let elem = tokio::time::timeout(Duration::from_secs(30), source.next())
            .await
            .expect("timed out waiting for message")
            .expect("source error");

        if let Some(StreamElement::Record(r)) = elem {
            received.push(r.value);
        }
    }

    received.sort();
    assert_eq!(
        received,
        vec![
            "message-0",
            "message-1",
            "message-2",
            "message-3",
            "message-4"
        ]
    );
}

#[tokio::test]
#[ignore]
async fn test_produce_with_key() {
    let (_container, brokers) = start_kafka().await;
    let topic = "test-produce-with-key";

    // Produce a keyed message
    {
        let mut sink: flume_connector_kafka::KafkaSink<String> = KafkaSinkBuilder::new()
            .brokers(&brokers)
            .topic(topic)
            .build()
            .unwrap();

        let record = Record::new("keyed-value".to_string(), EventTimestamp::new(1))
            .with_key(b"my-key".to_vec());
        sink.write(record).await.unwrap();
        sink.flush().await.unwrap();
    }

    // Consume and verify key is preserved
    let mut source = KafkaSourceBuilder::new()
        .brokers(&brokers)
        .topic(topic)
        .group_id("test-key-group")
        .config("auto.offset.reset", "earliest")
        .build()
        .unwrap();

    let elem = tokio::time::timeout(Duration::from_secs(30), source.next())
        .await
        .expect("timed out")
        .expect("source error")
        .expect("no message");

    if let StreamElement::Record(r) = elem {
        assert_eq!(r.value, "keyed-value");
        assert_eq!(r.key, Some(b"my-key".to_vec()));
    } else {
        panic!("expected Record");
    }
}
