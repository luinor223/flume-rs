//! Stream execution environment — the entry point for building pipelines.

use flume_core::{ChannelKind, PipelineConfig, Source};
use flume_runtime::dag::{LogicalGraph, NodeKind, PartitionStrategy};
use flume_runtime::scheduler::Scheduler;

use crate::datastream::{DataStream, SourceOrChannel};
use crate::source::InMemorySource;

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
}

impl Default for StreamExecutionEnvironment {
    fn default() -> Self {
        Self::new()
    }
}
