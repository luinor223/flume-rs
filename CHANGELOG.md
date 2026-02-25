# Changelog

## 0.1.0 — Initial Release

### Core Engine (`flume-core`)
- Generic `Record<T>`, `StreamElement<T>` with inline watermarks and checkpoint barriers
- `Operator`, `Collector`, `Source`, `Sink` traits
- Built-in operators: `Map`, `Filter`, `FlatMap`
- `EventTimestamp`, `PipelineConfig`, error types

### Runtime (`flume-runtime`)
- DAG-based logical graph with topological scheduling
- `TaskExecutor` hot loop with synchronous operator processing
- Bounded mpsc channels and lock-free SPSC ring buffers
- Hash/Forward/Broadcast/Rebalance partitioning strategies
- Watermark propagation across multi-input operators
- Chandy-Lamport checkpointing: `BarrierAligner`, `CheckpointCoordinator`, `SnapshotStore`
- Physical graph with Spread/Pack scheduling strategies
- CPU core pinning (behind `perf` feature)
- Criterion benchmarks for throughput, latency, and serde

### DataStream API (`flume-api`)
- Fluent builder: `map`, `filter`, `flat_map`, `key_by`, `window`, `aggregate`, `add_sink`
- `StreamExecutionEnvironment` with config and cancellation token support
- `InMemorySource`, `CollectionSource`, `PrintSink`

### Windowing
- `TumblingWindow`, `SlidingWindow` assigners
- `CountTrigger`, `EventTimeTrigger`, `ProcessingTimeTrigger`
- `WindowOperator` with key buffer pool optimization

### State (`flume-state`)
- `ValueState`, `ListState`, `MapState` traits
- `MemoryStateBackend` with per-key scoping
- State snapshot/restore via serialization

### Connectors
- **Kafka** — `KafkaSource` / `KafkaSink` with offset commit and flush-on-checkpoint
- **TCP** — `TcpSource` / `TcpSink` with line-delimited framing
- **File** — `FileSource` / `FileSink` for replay and rotating output

### Distributed Networking (`flume-network`)
- gRPC proto definitions for JobManager, TaskManager, and DataExchange services
- `JobManagerServiceImpl` / `TaskManagerServiceImpl` with event-driven architecture
- `JmClient` / `TmClient` for RPC calls
- `ResourceManager` for TM registration, heartbeats, and dead detection
- `ExchangeRouter` with credit-based flow control
- `NetworkCollector<T>` (serialization + batching) and `NetworkInput<T>` (deserialization)
- `ConnectionManager` for gRPC connection pooling
- `SlotManager` for TM execution slot lifecycle

### Server & CLI (`flume-server`)
- `flume-server` binary with REST API (axum) and optional JM gRPC mode
- `flume-tm` binary for standalone TaskManager processes
- `flume-cli` for job submission, status, listing, cancellation, and health checks
- Job registry pattern for compile-time job factory registration
- Job reaper for automatic status transitions
- TOML configuration with env var overlay
- `GET /api/v1/taskmanagers` endpoint for cluster visibility

### Production Hardening
- Structured logging with `tracing` (JSON output support)
- Prometheus metrics exposition
- Graceful shutdown via `CancellationToken` + SIGINT/SIGTERM
- `TestHarness` for operator unit testing
- Property-based tests with `proptest` for serde roundtrips

### CI/CD
- GitHub Actions: fmt, clippy, cargo-deny, check, test, integration tests, coverage
- Multi-stage Dockerfile with distroless runtime images
- GHCR publish workflow (tag/manual trigger)
