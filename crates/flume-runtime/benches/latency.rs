//! Single-record latency benchmark: measures the time for one record to pass
//! through a map operator.

use criterion::{Criterion, criterion_group, criterion_main};
use flume_core::{EventTimestamp, MapOperator, Record, StreamElement};
use flume_runtime::channel::operator_channel;
use flume_runtime::task::TaskExecutor;
use tokio::runtime::Runtime;

fn bench_single_record_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("single_record_latency", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
                let (output_collector, mut output_rx) = operator_channel::<i64>(16);

                let executor = TaskExecutor::new_mpsc(
                    "latency-map",
                    MapOperator::new(|x: i64| x + 1),
                    input_rx,
                    Box::new(output_collector),
                );

                let task = tokio::spawn(executor.run());

                // Send one record.
                input_tx
                    .send(StreamElement::Record(Record::new(
                        42i64,
                        EventTimestamp::new(1),
                    )))
                    .await
                    .unwrap();

                // Wait for the output.
                let _elem = output_rx.recv().await.unwrap();

                drop(input_tx);
                drop(output_rx);
                let _ = task.await;
            });
        });
    });
}

fn bench_single_record_latency_ring_buffer(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("single_record_latency_ring_buffer", |b| {
        b.iter(|| {
            rt.block_on(async {
                use flume_core::Collector;
                use flume_runtime::channel::operator_ring_buffer;

                let (mut input_collector, input) = operator_ring_buffer::<i64>(16);
                let (output_collector, mut output_rx) = operator_channel::<i64>(16);

                let executor = TaskExecutor::new(
                    "latency-map-rb",
                    MapOperator::new(|x: i64| x + 1),
                    input,
                    Box::new(output_collector),
                );

                let task = tokio::spawn(executor.run());

                // Send one record.
                input_collector
                    .collect(Record::new(42i64, EventTimestamp::new(1)))
                    .unwrap();

                // Wait for the output.
                let _elem = output_rx.recv().await.unwrap();

                drop(input_collector);
                drop(output_rx);
                let _ = task.await;
            });
        });
    });
}

criterion_group!(
    benches,
    bench_single_record_latency,
    bench_single_record_latency_ring_buffer,
);
criterion_main!(benches);
