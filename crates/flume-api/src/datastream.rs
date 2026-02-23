//! DataStream — a lazy, typed stream of records with fluent transformation API.

use flume_core::{
    CheckpointBarrier, Collector, FilterOperator, FlatMapOperator, FlumeError, FlumeResult,
    MapOperator, Operator, Record, Sink, Source, StreamElement, Watermark,
};
use flume_runtime::channel::operator_channel;
use flume_runtime::dag::{NodeKind, PartitionStrategy};
use flume_runtime::task::TaskExecutor;
use tokio::sync::mpsc;

use crate::environment::StreamExecutionEnvironment;

/// Internal representation of a DataStream's upstream input.
pub(crate) enum SourceOrChannel<T: Send + 'static> {
    Source(Box<dyn Source<T>>),
    Channel(mpsc::Receiver<StreamElement<T>>),
}

/// A lazy, typed stream of records. Transformations build the execution graph
/// but don't process data until a terminal operation ([`add_sink`](DataStream::add_sink))
/// is called.
pub struct DataStream<'env, T: Send + 'static> {
    env: &'env mut StreamExecutionEnvironment,
    node_id: usize,
    upstream: Option<SourceOrChannel<T>>,
}

impl<'env, T: Send + 'static> DataStream<'env, T> {
    pub(crate) fn new(
        env: &'env mut StreamExecutionEnvironment,
        node_id: usize,
        upstream: SourceOrChannel<T>,
    ) -> Self {
        Self {
            env,
            node_id,
            upstream: Some(upstream),
        }
    }

    /// Wire the upstream source or channel into an mpsc receiver.
    /// If upstream is a Source, spawns a drainer task that reads from the
    /// source and sends elements into the channel.
    fn wire_upstream(&mut self) -> mpsc::Receiver<StreamElement<T>> {
        let upstream = self.upstream.take().expect("upstream already consumed");
        match upstream {
            SourceOrChannel::Channel(rx) => rx,
            SourceOrChannel::Source(mut source) => {
                let buffer_size = self.env.config.channel_buffer_size;
                let (tx, rx) = mpsc::channel(buffer_size);
                self.env.scheduler.spawn("source-drainer", async move {
                    while let Some(element) = source.next().await? {
                        tx.send(element)
                            .await
                            .map_err(|_| FlumeError::ChannelClosed)?;
                    }
                    Ok(())
                });
                rx
            }
        }
    }

    /// Apply an operator transformation, spawning a TaskExecutor and returning
    /// a new DataStream over the output type.
    fn apply_operator<Out, Op>(mut self, name: &str, operator: Op) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        Op: Operator<T, Out> + 'static,
    {
        let input_rx = self.wire_upstream();
        let buffer_size = self.env.config.channel_buffer_size;
        let (output_collector, output_rx) = operator_channel::<Out>(buffer_size);

        let new_node_id = self.env.add_node(name, NodeKind::Operator, 1);
        self.env
            .add_edge(self.node_id, new_node_id, PartitionStrategy::Forward);

        let task_name = format!("{name}-{new_node_id}");
        let executor = TaskExecutor::new(
            task_name.clone(),
            operator,
            input_rx,
            Box::new(output_collector),
        );
        self.env.scheduler.spawn(task_name, executor.run());

        DataStream::new(self.env, new_node_id, SourceOrChannel::Channel(output_rx))
    }

    /// 1:1 transformation. Transforms each record via `f(T) -> U`.
    pub fn map<U, F>(self, f: F) -> DataStream<'env, U>
    where
        U: Send + 'static,
        F: FnMut(T) -> U + Send + 'static,
    {
        self.apply_operator("map", MapOperator::new(f))
    }

    /// Selective passthrough. Passes records where `predicate(&T)` is true.
    pub fn filter<F>(self, predicate: F) -> DataStream<'env, T>
    where
        F: FnMut(&T) -> bool + Send + 'static,
    {
        self.apply_operator("filter", FilterOperator::new(predicate))
    }

    /// 1:N expansion. Expands each record via `f(T) -> Vec<U>`.
    pub fn flat_map<U, F>(self, f: F) -> DataStream<'env, U>
    where
        U: Send + 'static,
        F: FnMut(T) -> Vec<U> + Send + 'static,
    {
        self.apply_operator("flat_map", FlatMapOperator::new(f))
    }

    /// Key-based partitioning. Attaches a partition key to each record
    /// by applying the key extractor and serializing the result to bytes.
    ///
    /// For now (single-node), this just sets the key on each record for
    /// downstream keyed operators. In distributed mode, records will be
    /// hash-partitioned across subtasks.
    pub fn key_by<K, F>(self, key_extractor: F) -> DataStream<'env, T>
    where
        K: Send + 'static + std::hash::Hash,
        F: FnMut(&T) -> K + Send + 'static,
    {
        self.apply_operator("key_by", KeyByOperator::new(key_extractor))
    }

    /// Terminal operation. Wires the stream to a sink and runs the pipeline.
    ///
    /// This consumes the DataStream and awaits completion of all tasks.
    pub async fn add_sink<S>(mut self, mut sink: S) -> FlumeResult<()>
    where
        S: Sink<T> + 'static,
    {
        let sink_id = self.env.add_node("sink", NodeKind::Sink, 1);
        self.env
            .add_edge(self.node_id, sink_id, PartitionStrategy::Forward);

        let mut input_rx = self.wire_upstream();

        // Drain the input channel inline (not spawned).
        while let Some(element) = input_rx.recv().await {
            match element {
                StreamElement::Record(record) => {
                    sink.write(record).await?;
                }
                StreamElement::Watermark(_) => {}
                StreamElement::CheckpointBarrier(_) => {
                    sink.flush().await?;
                }
            }
        }

        // Wait for all spawned tasks to finish.
        let scheduler = std::mem::take(&mut self.env.scheduler);
        scheduler.wait_all().await
    }
}

/// Internal operator that attaches a partition key to each record.
struct KeyByOperator<F, K> {
    key_extractor: F,
    _marker: std::marker::PhantomData<K>,
}

impl<F, K> KeyByOperator<F, K> {
    fn new(key_extractor: F) -> Self {
        Self {
            key_extractor,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, K, F> Operator<T, T> for KeyByOperator<F, K>
where
    T: Send,
    K: Send + std::hash::Hash,
    F: FnMut(&T) -> K + Send,
{
    fn process_record(
        &mut self,
        mut record: Record<T>,
        collector: &mut dyn Collector<T>,
    ) -> FlumeResult<()> {
        let key = (self.key_extractor)(&record.value);

        // Serialize key to bytes using a simple hash-based approach.
        // In a full implementation this would use proper serialization.
        let hash = {
            use std::hash::Hasher;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut hasher);
            hasher.finish().to_le_bytes().to_vec()
        };

        record.key = Some(hash);
        collector.collect(record)
    }

    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<T>,
    ) -> FlumeResult<()> {
        collector.collect_watermark(watermark)
    }

    fn process_barrier(
        &mut self,
        barrier: CheckpointBarrier,
        collector: &mut dyn Collector<T>,
    ) -> FlumeResult<()> {
        collector.collect_barrier(barrier)
    }
}

#[cfg(test)]
mod tests {
    use crate::sink::CollectSink;

    use super::*;

    #[tokio::test]
    async fn test_map_pipeline() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::new();

        env.from_collection(vec![1, 2, 3])
            .map(|x| x * 10)
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec![10, 20, 30]);
    }

    #[tokio::test]
    async fn test_filter_pipeline() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::new();

        env.from_collection(vec![1, 2, 3, 4, 5])
            .filter(|x| x % 2 == 0)
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec![2, 4]);
    }

    #[tokio::test]
    async fn test_flat_map_pipeline() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::new();

        env.from_collection(vec![1, 2, 3])
            .flat_map(|x| vec![x, x * 100])
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec![1, 100, 2, 200, 3, 300]);
    }

    #[tokio::test]
    async fn test_chained_transformations() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::new();

        env.from_collection(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
            .filter(|x| x % 2 == 0)
            .map(|x| x * 10)
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec![20, 40, 60, 80, 100]);
    }

    #[tokio::test]
    async fn test_key_by() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::new();

        env.from_collection(vec!["hello", "world"])
            .key_by(|s: &&str| s.len())
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec!["hello", "world"]);
    }

    #[tokio::test]
    async fn test_empty_collection() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::<i32>::new();

        env.from_collection(Vec::<i32>::new())
            .map(|x| x + 1)
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert!(values.is_empty());
    }
}
