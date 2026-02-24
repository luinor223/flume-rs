//! Test harness for unit testing operators in isolation.
//!
//! Wraps an [`Operator`] with a [`TestCollector`] so that you can feed
//! individual records, watermarks, or barriers and inspect the output
//! without wiring up channels or a scheduler.

use flume_core::{
    CheckpointBarrier, Collector, EventTimestamp, FlumeResult, Operator, Record, StreamElement,
    Watermark,
};

/// A [`Collector`] that stores all emitted elements in a `Vec`.
pub struct TestCollector<T> {
    elements: Vec<StreamElement<T>>,
}

impl<T> TestCollector<T> {
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }
}

impl<T> Default for TestCollector<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send> Collector<T> for TestCollector<T> {
    fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
        self.elements.push(StreamElement::Record(record));
        Ok(())
    }

    fn collect_watermark(&mut self, watermark: Watermark) -> FlumeResult<()> {
        self.elements.push(StreamElement::Watermark(watermark));
        Ok(())
    }

    fn collect_barrier(&mut self, barrier: CheckpointBarrier) -> FlumeResult<()> {
        self.elements
            .push(StreamElement::CheckpointBarrier(barrier));
        Ok(())
    }
}

/// A test harness that wraps an [`Operator`] for synchronous unit testing.
///
/// # Example
///
/// ```
/// use flume_core::{EventTimestamp, MapOperator};
/// use flume_runtime::test_harness::TestHarness;
///
/// let mut harness = TestHarness::new(MapOperator::new(|x: i32| x * 2));
/// harness.process_record_value(5, EventTimestamp::new(100)).unwrap();
/// assert_eq!(harness.take_values(), vec![10]);
/// ```
pub struct TestHarness<In: Send, Out: Send, Op: Operator<In, Out>> {
    operator: Op,
    collector: TestCollector<Out>,
    _marker: std::marker::PhantomData<In>,
}

impl<In: Send, Out: Send, Op: Operator<In, Out>> TestHarness<In, Out, Op> {
    pub fn new(operator: Op) -> Self {
        Self {
            operator,
            collector: TestCollector::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Process a full [`Record<In>`].
    pub fn process_record(&mut self, record: Record<In>) -> FlumeResult<()> {
        self.operator
            .process_record(record, &mut self.collector)
    }

    /// Convenience: process a value with a timestamp and no key.
    pub fn process_record_value(
        &mut self,
        value: In,
        timestamp: EventTimestamp,
    ) -> FlumeResult<()> {
        self.process_record(Record::new(value, timestamp))
    }

    /// Convenience: process a value with a timestamp and a pre-serialized key.
    pub fn process_keyed_record(
        &mut self,
        value: In,
        timestamp: EventTimestamp,
        key: Vec<u8>,
    ) -> FlumeResult<()> {
        self.process_record(Record::new(value, timestamp).with_key(key))
    }

    /// Send a watermark through the operator.
    pub fn process_watermark(&mut self, timestamp: EventTimestamp) -> FlumeResult<()> {
        self.operator
            .process_watermark(Watermark::new(timestamp), &mut self.collector)
    }

    /// Send a checkpoint barrier through the operator.
    pub fn process_barrier(&mut self, checkpoint_id: u64) -> FlumeResult<()> {
        self.operator.process_barrier(
            CheckpointBarrier::new(checkpoint_id, EventTimestamp::new(0)),
            &mut self.collector,
        )
    }

    /// Take all collected output, leaving the internal buffer empty.
    pub fn take_output(&mut self) -> Vec<StreamElement<Out>> {
        std::mem::take(&mut self.collector.elements)
    }

    /// Take only the record values from the output.
    pub fn take_records(&mut self) -> Vec<Record<Out>> {
        self.take_output()
            .into_iter()
            .filter_map(|e| match e {
                StreamElement::Record(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// Take only the record values (discarding timestamps/keys) from the output.
    pub fn take_values(&mut self) -> Vec<Out> {
        self.take_records().into_iter().map(|r| r.value).collect()
    }

    /// Peek at the current output without clearing it.
    pub fn peek_output(&self) -> &[StreamElement<Out>] {
        &self.collector.elements
    }

    /// Number of elements currently in the output buffer.
    pub fn output_len(&self) -> usize {
        self.collector.elements.len()
    }

    /// Get a shared reference to the operator (e.g., for state inspection).
    pub fn operator(&self) -> &Op {
        &self.operator
    }

    /// Get a mutable reference to the operator.
    pub fn operator_mut(&mut self) -> &mut Op {
        &mut self.operator
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use flume_core::{
        AggregateFunction, EventTimestamp, FilterOperator, FlatMapOperator, MapOperator,
        TumblingWindow, WindowOperator,
    };

    use super::*;

    #[test]
    fn test_harness_map() {
        let mut h = TestHarness::new(MapOperator::new(|x: i32| x * 3));

        h.process_record_value(10, EventTimestamp::new(0)).unwrap();
        h.process_record_value(20, EventTimestamp::new(1)).unwrap();

        assert_eq!(h.take_values(), vec![30, 60]);
    }

    #[test]
    fn test_harness_filter() {
        let mut h = TestHarness::new(FilterOperator::new(|x: &i32| *x > 5));

        h.process_record_value(3, EventTimestamp::new(0)).unwrap();
        h.process_record_value(7, EventTimestamp::new(1)).unwrap();
        h.process_record_value(2, EventTimestamp::new(2)).unwrap();

        assert_eq!(h.take_values(), vec![7]);
    }

    #[test]
    fn test_harness_flat_map() {
        let mut h = TestHarness::new(FlatMapOperator::new(|x: i32| vec![x, x + 1]));

        h.process_record_value(10, EventTimestamp::new(0)).unwrap();

        assert_eq!(h.take_values(), vec![10, 11]);
    }

    #[test]
    fn test_harness_watermark_forwarding() {
        let mut h = TestHarness::new(MapOperator::new(|x: i32| x));

        h.process_watermark(EventTimestamp::new(500)).unwrap();

        let output = h.take_output();
        assert_eq!(output.len(), 1);
        assert!(
            matches!(&output[0], StreamElement::Watermark(wm) if wm.timestamp == EventTimestamp::new(500))
        );
    }

    #[test]
    fn test_harness_barrier_forwarding() {
        let mut h = TestHarness::new(MapOperator::new(|x: i32| x));

        h.process_barrier(42).unwrap();

        let output = h.take_output();
        assert_eq!(output.len(), 1);
        assert!(
            matches!(&output[0], StreamElement::CheckpointBarrier(b) if b.checkpoint_id == 42)
        );
    }

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

    #[test]
    fn test_harness_window_operator() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let op = WindowOperator::new(Box::new(assigner), SumAgg);
        let mut h = TestHarness::new(op);

        // Feed keyed records in window [0, 10_000)
        h.process_keyed_record(10i64, EventTimestamp::new(1_000), vec![1])
            .unwrap();
        h.process_keyed_record(20i64, EventTimestamp::new(5_000), vec![1])
            .unwrap();
        h.process_keyed_record(30i64, EventTimestamp::new(8_000), vec![1])
            .unwrap();

        // No output until watermark fires
        assert_eq!(h.output_len(), 0);

        // Fire the window
        h.process_watermark(EventTimestamp::new(10_000)).unwrap();

        let values = h.take_values();
        assert_eq!(values, vec![60]); // 10 + 20 + 30
    }

    #[test]
    fn test_harness_peek_and_len() {
        let mut h = TestHarness::new(MapOperator::new(|x: i32| x));

        h.process_record_value(1, EventTimestamp::new(0)).unwrap();
        h.process_record_value(2, EventTimestamp::new(1)).unwrap();

        assert_eq!(h.output_len(), 2);
        assert_eq!(h.peek_output().len(), 2);

        // peek doesn't consume
        assert_eq!(h.output_len(), 2);

        // take does consume
        let _ = h.take_output();
        assert_eq!(h.output_len(), 0);
    }

    #[test]
    fn test_harness_operator_access() {
        let mut h = TestHarness::new(MapOperator::new(|x: i32| x));
        // Just verify we can access the operator
        let _op = h.operator();
        let _op_mut = h.operator_mut();
    }
}
