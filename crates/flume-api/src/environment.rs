//! Stream execution environment — the entry point for building pipelines.

use flume_core::{ChannelKind, PipelineConfig, Source};
use flume_runtime::dag::{LogicalGraph, NodeKind, PartitionStrategy};
use flume_runtime::scheduler::Scheduler;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::datastream::{DataStream, SourceOrChannel};
use crate::source::InMemorySource;

/// Initialize structured logging via `tracing-subscriber`.
///
/// Respects the `RUST_LOG` environment variable for filtering
/// (defaults to `info` if not set). When `json` is true, log output
/// uses JSON format suitable for structured log aggregation.
pub fn init_logging(json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Initialize Prometheus metrics exporter on the given bind address.
///
/// Starts an HTTP server serving `/metrics` in Prometheus exposition format.
/// Requires the `prometheus` feature.
#[cfg(feature = "prometheus")]
pub fn init_metrics(bind: std::net::SocketAddr) {
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(bind)
        .install()
        .expect("failed to install Prometheus exporter");
}

/// The entry point for building and executing a stream processing pipeline.
pub struct StreamExecutionEnvironment {
    pub(crate) config: PipelineConfig,
    pub(crate) graph: LogicalGraph,
    pub(crate) scheduler: Scheduler,
}

impl StreamExecutionEnvironment {
    pub fn new() -> Self {
        Self {
            config: PipelineConfig::default(),
            graph: LogicalGraph::new(),
            scheduler: Scheduler::new(),
        }
    }

    /// Create an environment from an existing [`PipelineConfig`].
    pub fn from_config(config: PipelineConfig) -> Self {
        Self {
            config,
            graph: LogicalGraph::new(),
            scheduler: Scheduler::new(),
        }
    }

    /// Create an environment by loading a TOML config file (with env var overlay).
    pub fn from_toml(path: &std::path::Path) -> flume_core::FlumeResult<Self> {
        let config = crate::config::load_config(Some(path))?;
        Ok(Self::from_config(config))
    }

    /// Set the default parallelism for operators.
    pub fn set_parallelism(&mut self, parallelism: usize) {
        self.config.parallelism = parallelism;
    }

    /// Set the bounded channel buffer size between operators.
    pub fn set_buffer_size(&mut self, buffer_size: usize) {
        self.config.channel_buffer_size = buffer_size;
    }

    /// Set the channel implementation between operators.
    pub fn set_channel_kind(&mut self, kind: ChannelKind) {
        self.config.channel_kind = kind;
    }

    /// Create a stream from an async [`Source`].
    pub fn from_source<T: Send + 'static>(
        &mut self,
        source: impl Source<T> + 'static,
    ) -> DataStream<'_, T> {
        let node_id = self.graph.add_node("source", NodeKind::Source, 1);
        DataStream::new(self, node_id, SourceOrChannel::Source(Box::new(source)))
    }

    /// Create a stream from a `Vec<T>` (for testing).
    pub fn from_collection<T: Send + 'static>(&mut self, items: Vec<T>) -> DataStream<'_, T> {
        self.from_source(InMemorySource::new(items))
    }

    /// Add an edge to the logical graph.
    pub(crate) fn add_edge(&mut self, from: usize, to: usize, strategy: PartitionStrategy) {
        self.graph.add_edge(from, to, strategy);
    }

    /// Add a node to the logical graph.
    pub(crate) fn add_node(&mut self, name: &str, kind: NodeKind, parallelism: usize) -> usize {
        self.graph.add_node(name, kind, parallelism)
    }

    /// Install a signal handler that triggers graceful shutdown on SIGINT/SIGTERM.
    ///
    /// Spawns a background task that listens for signals and calls
    /// `scheduler.cancel()` when received.
    pub fn install_signal_handler(&self) {
        let token = self.scheduler.cancel_token();
        tokio::spawn(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            {
                let mut sigterm =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("failed to install SIGTERM handler");
                tokio::select! {
                    _ = ctrl_c => {
                        info!("received SIGINT, initiating graceful shutdown");
                    }
                    _ = sigterm.recv() => {
                        info!("received SIGTERM, initiating graceful shutdown");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                ctrl_c.await.ok();
                info!("received Ctrl+C, initiating graceful shutdown");
            }
            token.cancel();
        });
    }
}

impl Default for StreamExecutionEnvironment {
    fn default() -> Self {
        Self::new()
    }
}
