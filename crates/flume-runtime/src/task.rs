//! TaskExecutor — the async hot loop that drives an operator.

use flume_core::{Collector, FlumeResult, Operator, StreamElement};
use tokio::sync::mpsc;

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
    pub input: mpsc::Receiver<StreamElement<In>>,
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
        input: mpsc::Receiver<StreamElement<In>>,
        collector: Box<dyn Collector<Out>>,
    ) -> Self {
        Self {
            name: name.into(),
            operator,
            input,
            collector,
        }
    }

    /// Run the hot loop until the input channel is closed.
    pub async fn run(mut self) -> FlumeResult<()> {
        while let Some(element) = self.input.recv().await {
            match element {
                StreamElement::Record(record) => {
                    self.operator
                        .process_record(record, self.collector.as_mut())?;
                }
                StreamElement::Watermark(wm) => {
                    self.operator
                        .process_watermark(wm, self.collector.as_mut())?;
                }
                StreamElement::CheckpointBarrier(barrier) => {
                    self.operator
                        .process_barrier(barrier, self.collector.as_mut())?;
                }
            }
        }
        Ok(())
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

        let executor = TaskExecutor::new(
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

        let executor = TaskExecutor::new(
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
        assert!(matches!(elem, StreamElement::Watermark(wm) if wm.timestamp == EventTimestamp::new(500)));

        handle.await.unwrap().unwrap();
    }
}
