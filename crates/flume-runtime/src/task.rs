//! TaskExecutor — the async hot loop that drives an operator.

use flume_core::{Collector, FlumeResult, Operator, StreamElement};
use tokio::sync::mpsc;
use tracing::{debug, info, info_span, trace, Instrument};

use crate::channel::OperatorInput;

/// Wraps a synchronous operator in an async task that reads from a channel
/// and dispatches elements to the operator.
pub struct TaskExecutor<In, Out, Op>
where
    In: Send + 'static,
    Out: Send + 'static,
    Op: Operator<In, Out>,
{
    pub name: String,
    pub operator: Op,
    pub input: OperatorInput<In>,
    pub collector: Box<dyn Collector<Out>>,
}

impl<In, Out, Op> TaskExecutor<In, Out, Op>
where
    In: Send + 'static,
    Out: Send + 'static,
    Op: Operator<In, Out>,
{
    pub fn new(
        name: impl Into<String>,
        operator: Op,
        input: OperatorInput<In>,
        collector: Box<dyn Collector<Out>>,
    ) -> Self {
        Self {
            name: name.into(),
            operator,
            input,
            collector,
        }
    }

    /// Create a TaskExecutor with an mpsc receiver (convenience for backward compat).
    pub fn new_mpsc(
        name: impl Into<String>,
        operator: Op,
        input: mpsc::Receiver<StreamElement<In>>,
        collector: Box<dyn Collector<Out>>,
    ) -> Self {
        Self::new(name, operator, OperatorInput::Mpsc(input), collector)
    }

    /// Run the hot loop until the input channel is closed.
    pub async fn run(mut self) -> FlumeResult<()> {
        let span = info_span!("task", name = %self.name);
        async {
            info!("task started");
            let mut records_processed: u64 = 0;
            while let Some(element) = self.input.recv().await {
                match element {
                    StreamElement::Record(record) => {
                        trace!(timestamp = record.timestamp.as_millis(), "processing record");
                        self.operator
                            .process_record(record, self.collector.as_mut())?;
                        records_processed += 1;
                    }
                    StreamElement::Watermark(wm) => {
                        debug!(timestamp = wm.timestamp.as_millis(), "watermark");
                        self.operator
                            .process_watermark(wm, self.collector.as_mut())?;
                    }
                    StreamElement::CheckpointBarrier(barrier) => {
                        info!(checkpoint_id = barrier.checkpoint_id, "barrier");
                        self.operator
                            .process_barrier(barrier, self.collector.as_mut())?;
                    }
                }
            }
            info!(records_processed, "task finished");
            Ok(())
        }
        .instrument(span)
        .await
    }
}

#[cfg(test)]
mod tests {
    use flume_core::{EventTimestamp, MapOperator, Record};

    use super::*;
    use crate::channel::operator_channel;

    #[tokio::test]
    async fn test_task_executor_processes_records() {
        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);

        let executor = TaskExecutor::new_mpsc(
            "test-map",
            MapOperator::new(|x: i32| x * 3),
            input_rx,
            Box::new(output_collector),
        );

        let handle = tokio::spawn(executor.run());

        input_tx
            .send(StreamElement::Record(Record::new(
                5,
                EventTimestamp::new(100),
            )))
            .await
            .unwrap();
        input_tx
            .send(StreamElement::Record(Record::new(
                10,
                EventTimestamp::new(200),
            )))
            .await
            .unwrap();
        drop(input_tx);

        let r1 = output_rx.recv().await.unwrap();
        assert!(matches!(r1, StreamElement::Record(r) if r.value == 15));

        let r2 = output_rx.recv().await.unwrap();
        assert!(matches!(r2, StreamElement::Record(r) if r.value == 30));

        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_task_executor_forwards_watermarks() {
        use flume_core::Watermark;

        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);

        let executor = TaskExecutor::new_mpsc(
            "test-passthrough",
            MapOperator::new(|x: i32| x),
            input_rx,
            Box::new(output_collector),
        );

        let handle = tokio::spawn(executor.run());

        input_tx
            .send(StreamElement::Watermark(Watermark::new(
                EventTimestamp::new(500),
            )))
            .await
            .unwrap();
        drop(input_tx);

        let elem = output_rx.recv().await.unwrap();
        assert!(
            matches!(elem, StreamElement::Watermark(wm) if wm.timestamp == EventTimestamp::new(500))
        );

        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_task_executor_with_ring_buffer() {
        use crate::channel::operator_ring_buffer;

        let (mut input_collector, input) = operator_ring_buffer::<i32>(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);

        let executor = TaskExecutor::new(
            "test-ring-map",
            MapOperator::new(|x: i32| x * 2),
            input,
            Box::new(output_collector),
        );

        let handle = tokio::spawn(executor.run());

        use flume_core::Collector;
        input_collector
            .collect(Record::new(7, EventTimestamp::new(100)))
            .unwrap();
        drop(input_collector);

        let r1 = output_rx.recv().await.unwrap();
        assert!(matches!(r1, StreamElement::Record(r) if r.value == 14));

        handle.await.unwrap().unwrap();
    }
}
