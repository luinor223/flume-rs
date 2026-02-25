# flume-rs

[![CI](https://github.com/luinor223/flume-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/luinor223/flume-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/luinor223/flume-rs/graph/badge.svg?token=1ZZMWBNX1H)](https://codecov.io/gh/luinor223/flume-rs)
[![Rust: 1.93+](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A distributed stream processing engine in Rust, inspired by [Apache Flink](https://flink.apache.org/). Targets sub-millisecond latency with zero GC pauses.

## Highlights

- Fluent `DataStream` API: `map`, `filter`, `flat_map`, `key_by`, `window`, `aggregate`
- Event-time processing with watermarks, tumbling/sliding/session windows, and late data handling
- Exactly-once state (`ValueState`, `ListState`, `MapState`) with Chandy-Lamport checkpointing
- Kafka, TCP, and file connectors out of the box
- Distributed execution via JobManager/TaskManager architecture with gRPC control plane
- Lock-free ring buffers, CPU pinning, zero-copy serialization (rkyv), >1M records/sec/core
- Production ready: structured logging, Prometheus metrics, TOML config, graceful shutdown

## Quick Start

### Run the server (standalone mode)

```bash
cargo run --release --bin flume-server
```

### Submit a job via CLI

```console
$ cargo run --release --bin flume-cli --features cli -- submit example-doubler
Job submitted: id=abc123 name=example-doubler status=running

$ cargo run --release --bin flume-cli --features cli -- list
ID        NAME              STATUS
abc123    example-doubler   completed
```

### Run in distributed mode

```bash
# Start JobManager with gRPC
cargo run --release --bin flume-server -- --mode jm --grpc-port 50051

# Start TaskManager(s)
cargo run --release --bin flume-tm -- --jm-address http://127.0.0.1:50051 --slots 4
```

### Docker

```bash
# Build images
docker build --target flume-server -t flume-server .
docker build --target flume-tm -t flume-tm .
docker build --target flume-cli -t flume-cli .

# Run
docker run -p 8080:8080 -p 50051:50051 flume-server --mode jm
docker run flume-tm --jm-address http://host.docker.internal:50051
```

## Programming Model

```rust
let mut env = StreamExecutionEnvironment::new();

env.from_source(KafkaSource::builder().topic("events").build())
   .map(|raw: Bytes| Event::decode(&raw))
   .filter(|e| e.value > 0)
   .key_by(|e| e.key.clone())
   .window(TumblingWindow::new(Duration::from_secs(60)))
   .aggregate(SumAggregator)
   .add_sink(PrintSink::new("output"));

env.execute("my-job").await?;
```

## REST API

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/jobs` | Submit a job |
| GET | `/api/v1/jobs` | List jobs |
| GET | `/api/v1/jobs/{id}` | Get job status |
| POST | `/api/v1/jobs/{id}/cancel` | Cancel a job |
| GET | `/api/v1/taskmanagers` | List TaskManagers |
| GET | `/health/live` | Liveness probe |
| GET | `/health/ready` | Readiness probe |

## Building

```bash
cargo build --workspace                    # Build all
cargo test --workspace                     # Run tests
cargo clippy --workspace --all-targets     # Lint
cargo fmt --check                          # Check formatting
```

Requires Rust 1.93+ and `protoc` (for gRPC code generation).

## License

[MIT](LICENSE)
