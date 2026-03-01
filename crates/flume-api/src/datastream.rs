//! DataStream — a lazy, typed stream of records with fluent transformation API.

use flume_core::{
    CheckpointBarrier, Collector, FilterOperator, FlatMapOperator, FlumeError, FlumeResult,
    KeyedProcessFunction, KeyedProcessOperator, MapOperator, Operator, ProcessFunction,
    ProcessOperator, Record, Sink, Source, StreamElement, Watermark, WindowAssigner,
};
use flume_runtime::channel::{OperatorInput, operator_channel_with_kind};
use flume_runtime::dag::{NodeKind, PartitionStrategy};
use flume_runtime::task::TaskExecutor;
use tracing::{debug, info};

use crate::connected::ConnectedStream;
use crate::environment::StreamExecutionEnvironment;
use crate::interval::IntervalJoinStream;
use crate::join::JoinStream;
use crate::windowed::WindowedStream;

/// Internal representation of a DataStream's upstream input.
pub(crate) enum SourceOrChannel<T: Send + 'static> {
    Source(Box<dyn Source<T>>),
    Input(OperatorInput<T>),
}

/// A lazy, typed stream of records. Transformations build the execution graph
/// but don't process data until a terminal operation ([`add_sink`](DataStream::add_sink))
/// is called.
pub struct DataStream<'env, T: Send + 'static> {
    pub(crate) env: &'env mut StreamExecutionEnvironment,
    pub(crate) node_id: usize,
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

    /// Wire the upstream source or channel into an `OperatorInput`.
    /// If upstream is a Source, spawns a drainer task that reads from the
    /// source and sends elements into an mpsc channel.
    pub(crate) fn wire_upstream(&mut self) -> OperatorInput<T> {
        let upstream = self.upstream.take().expect("upstream already consumed");
        match upstream {
            SourceOrChannel::Input(input) => input,
            SourceOrChannel::Source(mut source) => {
                let buffer_size = self.env.config.channel_buffer_size;
                let (tx, rx) = tokio::sync::mpsc::channel(buffer_size);
                info!("spawning source drainer");
                self.env.scheduler.spawn("source-drainer", async move {
                    while let Some(element) = source.next().await? {
                        tx.send(element)
                            .await
                            .map_err(|_| FlumeError::ChannelClosed)?;
                    }
                    Ok(())
                });
                OperatorInput::Mpsc(rx)
            }
        }
    }

    /// Apply an operator transformation, spawning a TaskExecutor and returning
    /// a new DataStream over the output type.
    pub(crate) fn apply_operator<Out, Op>(
        mut self,
        name: &str,
        operator: Op,
    ) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        Op: Operator<T, Out> + 'static,
    {
        let input = self.wire_upstream();
        let buffer_size = self.env.config.channel_buffer_size;
        let channel_kind = self.env.config.channel_kind;
        let (output_collector, output_input) =
            operator_channel_with_kind::<Out>(buffer_size, channel_kind);

        let new_node_id = self.env.add_node(name, NodeKind::Operator, 1);
        self.env
            .add_edge(self.node_id, new_node_id, PartitionStrategy::Forward);

        let task_name = format!("{name}-{new_node_id}");
        debug!(operator = %name, node_id = new_node_id, "wiring operator");
        let cancel = self.env.scheduler.cancel_token();
        let executor = TaskExecutor::new(task_name.clone(), operator, input, output_collector)
            .with_cancel(cancel);
        self.env.scheduler.spawn(task_name, executor.run());

        DataStream::new(self.env, new_node_id, SourceOrChannel::Input(output_input))
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

    /// Apply a process function for record-by-record processing with timers.
    ///
    /// The most expressive transformation: can emit zero, one, or many
    /// outputs per input, and can register event-time timers.
    pub fn process<Out, PF>(self, process_fn: PF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        PF: ProcessFunction<T, Out> + 'static,
    {
        self.apply_operator("process", ProcessOperator::new(process_fn))
    }

    /// Apply a keyed process function for per-key processing with timers.
    ///
    /// Must be called after `key_by()`. Receives the partition key alongside
    /// each record for per-key stateful processing.
    pub fn process_keyed<Out, PF>(self, process_fn: PF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        PF: KeyedProcessFunction<T, Out> + 'static,
    {
        self.apply_operator("keyed_process", KeyedProcessOperator::new(process_fn))
    }

    /// Connect this stream with a second input source for two-input
    /// co-processing. Returns a [`ConnectedStream`] on which you call
    /// `.process()` with a [`CoProcessFunction`](flume_core::CoProcessFunction).
    ///
    /// The second input is provided as a [`Source`] rather than a
    /// `DataStream` to avoid borrow-checker issues (both streams would
    /// need `&mut env`).
    pub fn connect<U: Send + 'static>(
        self,
        source: impl Source<U> + 'static,
    ) -> ConnectedStream<'env, T, U> {
        ConnectedStream::new(self, Box::new(source))
    }

    /// Join this stream with a second input source using windowed join.
    /// Returns a [`JoinStream`] for configuring keys and windows.
    pub fn window_join<U: Send + 'static>(
        self,
        source: impl Source<U> + 'static,
    ) -> JoinStream<'env, T, U> {
        JoinStream::new(self, Box::new(source))
    }

    /// Join this stream with a second input source using interval join.
    /// Returns an [`IntervalJoinStream`] for configuring keys and bounds.
    pub fn interval_join<U: Send + 'static>(
        self,
        source: impl Source<U> + 'static,
    ) -> IntervalJoinStream<'env, T, U> {
        IntervalJoinStream::new(self, Box::new(source))
    }

    /// Assign records to windows. Must be called after `key_by`.
    /// Returns a [`WindowedStream`] on which you call `.aggregate()`.
    pub fn window(self, assigner: impl WindowAssigner + 'static) -> WindowedStream<'env, T> {
        WindowedStream::new(self, Box::new(assigner))
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

        let mut input = self.wire_upstream();

        // Drain the input channel inline (not spawned).
        info!("sink drain started");
        while let Some(element) = input.recv().await {
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

        info!("sink drain finished");
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
    use crate::source::TimestampedSource;
    use flume_core::{AggregateFunction, EventTimestamp, TumblingWindow};
    use std::time::Duration;

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

    /// Sum aggregator for windowing test.
    struct SumAgg;

    impl AggregateFunction<i64, i64, i64> for SumAgg {
        fn create_accumulator(&self) -> i64 {
            0
        }
        fn add(&self, acc: &mut i64, value: &i64) {
            *acc += value;
        }
        fn get_result(&self, acc: &i64) -> i64 {
            *acc
        }
        fn merge(&self, a: &mut i64, b: i64) {
            *a += b;
        }
    }

    #[tokio::test]
    async fn test_map_pipeline_ring_buffer() {
        use flume_core::ChannelKind;

        let mut env = StreamExecutionEnvironment::new();
        env.set_channel_kind(ChannelKind::RingBuffer);
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
    async fn test_chained_ring_buffer() {
        use flume_core::ChannelKind;

        let mut env = StreamExecutionEnvironment::new();
        env.set_channel_kind(ChannelKind::RingBuffer);
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
    async fn test_windowed_pipeline() {
        let mut env = StreamExecutionEnvironment::new();
        let (sink, results) = CollectSink::<i64>::new();

        // Build elements: 3 records in window [0, 10_000), then a watermark to fire it.
        let elements = vec![
            StreamElement::Record(Record::new(10i64, EventTimestamp::new(1_000)).with_key(vec![1])),
            StreamElement::Record(Record::new(20i64, EventTimestamp::new(5_000)).with_key(vec![1])),
            StreamElement::Record(Record::new(30i64, EventTimestamp::new(8_000)).with_key(vec![1])),
            StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
        ];

        let source = TimestampedSource::new(elements);

        env.from_source(source)
            .window(TumblingWindow::new(Duration::from_secs(10)))
            .aggregate(SumAgg)
            .add_sink(sink)
            .await
            .unwrap();

        let values = results.lock().unwrap();
        assert_eq!(*values, vec![60]); // 10 + 20 + 30
    }
}
