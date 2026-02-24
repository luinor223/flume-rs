//! Benchmarks comparing mpsc vs ring buffer channel throughput.

use criterion::{Criterion, criterion_group, criterion_main};
use flume_core::{Collector, EventTimestamp, Record};
use flume_runtime::channel::{OperatorInput, operator_channel, operator_ring_buffer};
use flume_runtime::ring_buffer;
use tokio::runtime::Runtime;

fn bench_mpsc_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let count: i64 = 10_000;

    c.bench_function("mpsc_send_recv_10k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut tx, rx) = operator_channel::<i64>(1024);
                let mut input = OperatorInput::Mpsc(rx);

                let sender = tokio::spawn(async move {
                    for i in 0..count {
                        tx.collect(Record::new(i, EventTimestamp::new(i as u64)))
                            .unwrap();
                    }
                });

                let mut n = 0i64;
                while let Some(_elem) = input.recv().await {
                    n += 1;
                    if n >= count {
                        break;
                    }
                }

                sender.await.unwrap();
            });
        });
    });
}

fn bench_ring_buffer_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let count: i64 = 10_000;

    c.bench_function("ring_buffer_send_recv_10k", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (mut tx, mut input) = operator_ring_buffer::<i64>(1024);

                let sender = tokio::spawn(async move {
                    for i in 0..count {
                        loop {
                            match tx.collect(Record::new(i, EventTimestamp::new(i as u64))) {
                                Ok(()) => break,
                                Err(_) => tokio::task::yield_now().await,
                            }
                        }
                    }
                });

                let mut n = 0i64;
                while let Some(_elem) = input.recv().await {
                    n += 1;
                    if n >= count {
                        break;
                    }
                }

                sender.await.unwrap();
            });
        });
    });
}

fn bench_ring_buffer_try_send(c: &mut Criterion) {
    c.bench_function("ring_buffer_try_send_recv_1m", |b| {
        b.iter(|| {
            let (mut producer, mut consumer) = ring_buffer::ring_buffer::<i64>(1024);
            let count = 1_000_000i64;

            for i in 0..count {
                loop {
                    match producer.try_send(i) {
                        Ok(()) => break,
                        Err(_) => {
                            // Drain some to make room.
                            while consumer.try_recv().is_some() {}
                        }
                    }
                }
            }
            // Drain remaining.
            while consumer.try_recv().is_some() {}
        });
    });
}

criterion_group!(
    benches,
    bench_mpsc_throughput,
    bench_ring_buffer_throughput,
    bench_ring_buffer_try_send,
);
criterion_main!(benches);
