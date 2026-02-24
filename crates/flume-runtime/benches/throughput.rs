//! Pipeline throughput benchmark: measures end-to-end records/sec through a map operator.

use criterion::{Criterion, criterion_group, criterion_main};
use flume_core::{Collector, EventTimestamp, MapOperator, Record, StreamElement};
use flume_runtime::channel::{operator_channel, operator_ring_buffer};
use flume_runtime::task::TaskExecutor;
use tokio::runtime::Runtime;

fn bench_map_pipeline_mpsc(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let count: i64 = 100_000;

    c.bench_function("map_pipeline_mpsc_100k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (input_tx, input_rx) = tokio::sync::mpsc::channel(1024);
                let (output_collector, mut output_rx) = operator_channel::<i64>(1024);

                let executor = TaskExecutor::new_mpsc(
                    "bench-map",
                    MapOperator::new(|x: i64| x * 2 + 1),
                    input_rx,
                    Box::new(output_collector),
                );

                let task = tokio::spawn(executor.run());

                let sender = tokio::spawn(async move {
                    for i in 0..count {
                        input_tx
                            .send(StreamElement::Record(Record::new(
                                i,
                                EventTimestamp::new(i as u64),
                            )))
                            .await
                            .unwrap();
                    }
                    // input_tx is dropped here, closing the channel
                });

                let mut received = 0i64;
                while received < count {
                    if output_rx.recv().await.is_some() {
                        received += 1;
                    }
                }

                sender.await.unwrap();
                let _ = task.await;
            });
        });
    });
}

fn bench_map_pipeline_ring_buffer(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let count: i64 = 100_000;

    c.bench_function("map_pipeline_ring_buffer_100k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut input_collector, input) = operator_ring_buffer::<i64>(1024);
                let (output_collector, mut output_rx) = operator_channel::<i64>(1024);

                let executor = TaskExecutor::new(
                    "bench-map-rb",
                    MapOperator::new(|x: i64| x * 2 + 1),
                    input,
                    Box::new(output_collector),
                );

                let task = tokio::spawn(executor.run());

                let sender = tokio::spawn(async move {
                    for i in 0..count {
                        loop {
                            match input_collector
                                .collect(Record::new(i, EventTimestamp::new(i as u64)))
                            {
                                Ok(()) => break,
                                Err(_) => tokio::task::yield_now().await,
                            }
                        }
                    }
                    // input_collector is dropped here
                });

                let mut received = 0i64;
                while received < count {
                    if output_rx.recv().await.is_some() {
                        received += 1;
                    }
                }

                sender.await.unwrap();
                let _ = task.await;
            });
        });
    });
}

criterion_group!(
    benches,
    bench_map_pipeline_mpsc,
    bench_map_pipeline_ring_buffer
);
criterion_main!(benches);
