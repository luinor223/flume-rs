//! Benchmark for bitcode serialization/deserialization used in state backends.

use criterion::{Criterion, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct SmallRecord {
    id: u64,
    value: f64,
    tag: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct LargeRecord {
    id: u64,
    values: Vec<f64>,
    tags: Vec<String>,
    metadata: std::collections::HashMap<String, String>,
}

fn bench_bitcode_small_serialize(c: &mut Criterion) {
    let record = SmallRecord {
        id: 42,
        value: 2.71,
        tag: "test".to_string(),
    };

    c.bench_function("bitcode_small_serialize", |b| {
        b.iter(|| {
            let _bytes = bitcode::serialize(&record).unwrap();
        });
    });
}

fn bench_bitcode_small_deserialize(c: &mut Criterion) {
    let record = SmallRecord {
        id: 42,
        value: 2.71,
        tag: "test".to_string(),
    };
    let bytes = bitcode::serialize(&record).unwrap();

    c.bench_function("bitcode_small_deserialize", |b| {
        b.iter(|| {
            let _r: SmallRecord = bitcode::deserialize(&bytes).unwrap();
        });
    });
}

fn bench_bitcode_large_serialize(c: &mut Criterion) {
    let record = LargeRecord {
        id: 1,
        values: (0..100).map(|i| i as f64 * 1.1).collect(),
        tags: (0..20).map(|i| format!("tag-{i}")).collect(),
        metadata: (0..10)
            .map(|i| (format!("key-{i}"), format!("value-{i}")))
            .collect(),
    };

    c.bench_function("bitcode_large_serialize", |b| {
        b.iter(|| {
            let _bytes = bitcode::serialize(&record).unwrap();
        });
    });
}

fn bench_bitcode_large_deserialize(c: &mut Criterion) {
    let record = LargeRecord {
        id: 1,
        values: (0..100).map(|i| i as f64 * 1.1).collect(),
        tags: (0..20).map(|i| format!("tag-{i}")).collect(),
        metadata: (0..10)
            .map(|i| (format!("key-{i}"), format!("value-{i}")))
            .collect(),
    };
    let bytes = bitcode::serialize(&record).unwrap();

    c.bench_function("bitcode_large_deserialize", |b| {
        b.iter(|| {
            let _r: LargeRecord = bitcode::deserialize(&bytes).unwrap();
        });
    });
}

fn bench_bitcode_roundtrip_batch(c: &mut Criterion) {
    let records: Vec<SmallRecord> = (0..1000)
        .map(|i| SmallRecord {
            id: i,
            value: i as f64 * 0.5,
            tag: format!("item-{i}"),
        })
        .collect();

    c.bench_function("bitcode_batch_1k_roundtrip", |b| {
        b.iter(|| {
            let bytes = bitcode::serialize(&records).unwrap();
            let _r: Vec<SmallRecord> = bitcode::deserialize(&bytes).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_bitcode_small_serialize,
    bench_bitcode_small_deserialize,
    bench_bitcode_large_serialize,
    bench_bitcode_large_deserialize,
    bench_bitcode_roundtrip_batch,
);
criterion_main!(benches);
