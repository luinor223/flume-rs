//! ConnectedStream — a two-input stream ready for co-processing.
//!
//! Created by [`DataStream::connect`]. Merges two inputs through
//! [`TwoInputMerger`](flume_runtime::merge::two_input_merge), tags elements
//! as `Either<In1, In2>`, and feeds them to a [`CoProcessOperator`].

use std::hash::Hash;

use flume_core::{
    CoProcessFunction, CoProcessOperator, Either, FlumeError, Source, StreamElement,
};
use flume_runtime::channel::{OperatorInput, operator_channel_with_kind};
use flume_runtime::dag::{NodeKind, PartitionStrategy};
use flume_runtime::merge::two_input_merge;
use flume_runtime::task::TaskExecutor;
use tokio::sync::mpsc;
use tracing::debug;

use crate::datastream::{DataStream, SourceOrChannel};

type KeyExtractor<T> = Box<dyn FnMut(&T) -> Vec<u8> + Send>;

/// A two-input stream created by connecting a `DataStream<In1>` with a
/// `Source<In2>`. Call [`process`](Self::process) to apply a
/// [`CoProcessFunction`] and produce a `DataStream<Out>`.
pub struct ConnectedStream<'env, In1: Send + 'static, In2: Send + 'static> {
    stream: DataStream<'env, In1>,
    source: Box<dyn Source<In2>>,
    key_extractor_1: Option<KeyExtractor<In1>>,
    key_extractor_2: Option<KeyExtractor<In2>>,
}

impl<'env, In1: Send + 'static, In2: Send + 'static> ConnectedStream<'env, In1, In2> {
    pub(crate) fn new(stream: DataStream<'env, In1>, source: Box<dyn Source<In2>>) -> Self {
        Self {
            stream,
            source,
            key_extractor_1: None,
            key_extractor_2: None,
        }
    }

    /// Set key extractors for both inputs. Keys are hashed to bytes using
    /// the same approach as `DataStream::key_by`.
    pub fn key_by_both<K1, K2, F1, F2>(mut self, mut f1: F1, mut f2: F2) -> Self
    where
        K1: Hash + Send + 'static,
        K2: Hash + Send + 'static,
        F1: FnMut(&In1) -> K1 + Send + 'static,
        F2: FnMut(&In2) -> K2 + Send + 'static,
    {
        self.key_extractor_1 = Some(Box::new(move |value: &In1| {
            hash_key(&f1(value))
        }));
        self.key_extractor_2 = Some(Box::new(move |value: &In2| {
            hash_key(&f2(value))
        }));
        self
    }

    /// Apply a [`CoProcessFunction`] to the connected streams, producing a
    /// new `DataStream<Out>`.
    ///
    /// Internally:
    /// 1. Wires stream1's upstream into an mpsc channel
    /// 2. Spawns a drainer for source2 into an mpsc channel
    /// 3. Applies key extractors inline if set
    /// 4. Spawns a `two_input_merge` task to merge both into `Either<In1, In2>`
    /// 5. Spawns a `TaskExecutor` with the `CoProcessOperator`
    pub fn process<Out, CPF>(mut self, co_process_fn: CPF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        CPF: CoProcessFunction<In1, In2, Out> + 'static,
    {
        let buffer_size = self.stream.env.config.channel_buffer_size;

        // Wire stream1 upstream → mpsc channel for In1.
        let input1 = self.stream.wire_upstream();
        let (tx1, rx1) = mpsc::channel::<StreamElement<In1>>(buffer_size);

        // Spawn task to drain input1 (applying key extractor if set).
        let mut key_fn_1 = self.key_extractor_1.take();
        self.stream.env.scheduler.spawn("connected-input1", async move {
            let mut input = input1;
            while let Some(mut element) = input.recv().await {
                if let Some(ref mut key_fn) = key_fn_1
                    && let StreamElement::Record(ref mut record) = element
                {
                    record.key = Some(key_fn(&record.value));
                }
                tx1.send(element)
                    .await
                    .map_err(|_| FlumeError::ChannelClosed)?;
            }
            Ok(())
        });

        // Wire source2 → mpsc channel for In2.
        let (tx2, rx2) = mpsc::channel::<StreamElement<In2>>(buffer_size);
        let mut source2 = self.source;
        let mut key_fn_2 = self.key_extractor_2.take();
        self.stream.env.scheduler.spawn("connected-input2", async move {
            while let Some(mut element) = source2.next().await? {
                if let Some(ref mut key_fn) = key_fn_2
                    && let StreamElement::Record(ref mut record) = element
                {
                    record.key = Some(key_fn(&record.value));
                }
                tx2.send(element)
                    .await
                    .map_err(|_| FlumeError::ChannelClosed)?;
            }
            Ok(())
        });

        // Merged channel: Either<In1, In2>.
        let (merged_tx, merged_rx) = mpsc::channel::<StreamElement<Either<In1, In2>>>(buffer_size);

        // Spawn the two-input merge task.
        self.stream.env.scheduler.spawn("two-input-merge", async move {
            two_input_merge(rx1, rx2, merged_tx).await
        });

        // Create the CoProcessOperator and wire it via TaskExecutor.
        let operator = CoProcessOperator::new(co_process_fn);
        let merged_input = OperatorInput::Mpsc(merged_rx);

        let channel_kind = self.stream.env.config.channel_kind;
        let (output_collector, output_input) =
            operator_channel_with_kind::<Out>(buffer_size, channel_kind);

        let new_node_id = self.stream.env.add_node("co_process", NodeKind::Operator, 1);
        self.stream.env.add_edge(
            self.stream.node_id,
            new_node_id,
            PartitionStrategy::Forward,
        );

        let task_name = format!("co_process-{new_node_id}");
        debug!(operator = "co_process", node_id = new_node_id, "wiring co-process operator");
        let cancel = self.stream.env.scheduler.cancel_token();
        let executor =
            TaskExecutor::new(task_name.clone(), operator, merged_input, output_collector)
                .with_cancel(cancel);
        self.stream.env.scheduler.spawn(task_name, executor.run());

        DataStream::new(
            self.stream.env,
            new_node_id,
            SourceOrChannel::Input(output_input),
        )
    }
}

/// Hash a key to bytes using the same approach as `KeyByOperator`.
fn hash_key(key: &impl Hash) -> Vec<u8> {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish().to_le_bytes().to_vec()
}

