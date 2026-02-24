//! TaskExecutor — the async hot loop that drives an operator.

use flume_core::{Collector, FlumeResult, Operator, StreamElement};
use metrics::{counter, histogram};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, info, info_span, trace};

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
    cancel: Option<CancellationToken>,
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
            cancel: None,
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

    /// Attach a cancellation token for graceful shutdown.
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Run the hot loop until the input channel is closed or cancellation is requested.
    ///
    /// When cancelled, remaining buffered elements are drained before returning.
    pub async fn run(mut self) -> FlumeResult<()> {
        let span = info_span!("task", name = %self.name);
        let task_name = self.name.clone();
        let cancel = self.cancel.take();
        async {
            info!("task started");
            let mut records_processed: u64 = 0;
            let mut cancelled = false;
            loop {
                let element = if cancelled {
                    // After cancellation, drain remaining buffered elements.
                    self.input.recv().await
                } else if let Some(ref token) = cancel {
                    // Use biased select so cancellation is only checked when recv would block.
                    tokio::select! {
                        biased;
                        elem = self.input.recv() => elem,
                        _ = token.cancelled() => {
                            info!("cancellation received, draining remaining records");
                            cancelled = true;
                            continue;
                        }
                    }
                } else {
                    self.input.recv().await
                };

                let Some(element) = element else { break };

                match element {
                    StreamElement::Record(record) => {
                        trace!(
                            timestamp = record.timestamp.as_millis(),
                            "processing record"
                        );
                        let start = std::time::Instant::now();
                        self.operator
                            .process_record(record, self.collector.as_mut())?;
                        let elapsed_us = start.elapsed().as_micros() as f64;
                        counter!("flume.records.processed", "task" => task_name.clone())
                            .increment(1);
                        histogram!("flume.record.latency_us", "task" => task_name.clone())
                            .record(elapsed_us);
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
            counter!("flume.tasks.completed").increment(1);
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
    async fn test_task_executor_records_metrics() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        recorder.install().unwrap();

        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);

        let executor = TaskExecutor::new_mpsc(
            "metrics-test",
            MapOperator::new(|x: i32| x + 1),
            input_rx,
            Box::new(output_collector),
        );

        let handle = tokio::spawn(executor.run());

        input_tx
            .send(StreamElement::Record(Record::new(
                1,
                EventTimestamp::new(100),
            )))
            .await
            .unwrap();
        // Drain output so the task doesn't block.
        let _ = output_rx.recv().await;
        drop(input_tx);
        handle.await.unwrap().unwrap();

        let snapshot = snapshotter.snapshot();
        // Verify records.processed counter was incremented
        let key = metrics_util::CompositeKey::new(
            metrics_util::MetricKind::Counter,
            metrics::Key::from_parts(
                "flume.records.processed",
                vec![metrics::Label::new("task", "metrics-test")],
            ),
        );
        let counter_val = snapshot
            .into_vec()
            .into_iter()
            .find(|(k, _, _, _)| *k == key)
            .map(|(_, _, _, dv)| match dv {
                metrics_util::debugging::DebugValue::Counter(v) => v,
                _ => panic!("expected counter"),
            })
            .expect("counter not found");
        assert_eq!(counter_val, 1);
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

    #[tokio::test]
    async fn test_cancel_token_propagation() {
        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);
        let token = CancellationToken::new();

        let executor = TaskExecutor::new_mpsc(
            "cancel-test",
            MapOperator::new(|x: i32| x),
            input_rx,
            Box::new(output_collector),
        )
        .with_cancel(token.clone());

        let handle = tokio::spawn(executor.run());

        // Send a record, then cancel
        input_tx
            .send(StreamElement::Record(Record::new(
                42,
                EventTimestamp::new(100),
            )))
            .await
            .unwrap();

        // Let the task process the record
        let _ = output_rx.recv().await;

        // Cancel and drop the sender so the drain finishes
        token.cancel();
        drop(input_tx);

        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_cancellation_drains_remaining() {
        let (input_tx, input_rx) = tokio::sync::mpsc::channel(16);
        let (output_collector, mut output_rx) = operator_channel::<i32>(16);
        let token = CancellationToken::new();

        let executor = TaskExecutor::new_mpsc(
            "drain-test",
            MapOperator::new(|x: i32| x * 2),
            input_rx,
            Box::new(output_collector),
        )
        .with_cancel(token.clone());

        // Send records before starting the task
        input_tx
            .send(StreamElement::Record(Record::new(
                1,
                EventTimestamp::new(100),
            )))
            .await
            .unwrap();
        input_tx
            .send(StreamElement::Record(Record::new(
                2,
                EventTimestamp::new(200),
            )))
            .await
            .unwrap();

        let handle = tokio::spawn(executor.run());

        // Cancel immediately and close the channel
        token.cancel();
        drop(input_tx);

        handle.await.unwrap().unwrap();

        // All records should have been drained and processed
        let r1 = output_rx.recv().await.unwrap();
        assert!(matches!(r1, StreamElement::Record(r) if r.value == 2));
        let r2 = output_rx.recv().await.unwrap();
        assert!(matches!(r2, StreamElement::Record(r) if r.value == 4));
    }
}
