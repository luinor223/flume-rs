//! Window operator that buffers records per key+window and fires on watermark.

use std::collections::HashMap;

use crate::window::{AggregateFunction, EventTimeTrigger, Trigger, TriggerResult, WindowAssigner};
use crate::{
    CheckpointBarrier, Collector, EventTimestamp, FlumeResult, Operator, Record, Watermark, Window,
};

/// Key for the per-key, per-window accumulator map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WindowKey {
    /// Serialized partition key (from `key_by`).
    key: Vec<u8>,
    /// The window this accumulator belongs to.
    window: Window,
}

/// A stateful operator that assigns records to windows, incrementally
/// aggregates them, and emits results when the trigger fires.
pub struct WindowOperator<A, Agg>
where
    A: Send,
    Agg: Send,
{
    assigner: Box<dyn WindowAssigner>,
    trigger: Box<dyn Trigger>,
    aggregate: Agg,
    /// Per (key, window) accumulators.
    accumulators: HashMap<WindowKey, A>,
}

impl<A, Agg> WindowOperator<A, Agg>
where
    A: Send,
    Agg: Send,
{
    pub fn new(assigner: Box<dyn WindowAssigner>, aggregate: Agg) -> Self {
        Self {
            assigner,
            trigger: Box::new(EventTimeTrigger),
            aggregate,
            accumulators: HashMap::new(),
        }
    }

    pub fn with_trigger(mut self, trigger: Box<dyn Trigger>) -> Self {
        self.trigger = trigger;
        self
    }
}

impl<In, Acc, Out, Agg> Operator<In, Out> for WindowOperator<Acc, Agg>
where
    In: Send,
    Acc: Send,
    Out: Send,
    Agg: AggregateFunction<In, Acc, Out> + Send,
{
    fn process_record(
        &mut self,
        record: Record<In>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_default();
        let windows = self.assigner.assign_windows(record.timestamp);

        for window in windows {
            let wk = WindowKey {
                key: key.clone(),
                window,
            };
            let acc = self
                .accumulators
                .entry(wk)
                .or_insert_with(|| self.aggregate.create_accumulator());
            self.aggregate.add(acc, &record.value);
        }

        Ok(())
    }

    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        // Evaluate trigger for each active window.
        let mut to_fire = Vec::new();
        let mut to_purge = Vec::new();

        for wk in self.accumulators.keys() {
            let result = self.trigger.on_event_time(watermark.timestamp, &wk.window);
            match result {
                TriggerResult::Fire => to_fire.push(wk.clone()),
                TriggerResult::Purge => to_purge.push(wk.clone()),
                TriggerResult::FireAndPurge => {
                    to_fire.push(wk.clone());
                    to_purge.push(wk.clone());
                }
                TriggerResult::Continue => {}
            }
        }

        // Fire: emit aggregated results.
        for wk in &to_fire {
            if let Some(acc) = self.accumulators.get(wk) {
                let result = self.aggregate.get_result(acc);
                let out_record = Record {
                    value: result,
                    timestamp: EventTimestamp::new(wk.window.end.as_millis() - 1),
                    key: Some(wk.key.clone()),
                };
                collector.collect(out_record)?;
            }
        }

        // Purge: remove accumulators.
        for wk in &to_purge {
            self.accumulators.remove(wk);
        }

        // Forward the watermark.
        collector.collect_watermark(watermark)
    }

    fn process_barrier(
        &mut self,
        barrier: CheckpointBarrier,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        collector.collect_barrier(barrier)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::StreamElement;
    use crate::window::TumblingWindow;

    /// Simple sum aggregator for testing.
    struct SumAggregator;

    impl AggregateFunction<i64, i64, i64> for SumAggregator {
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

    /// Test collector that records everything emitted.
    struct TestCollector<T> {
        elements: Vec<StreamElement<T>>,
    }

    impl<T> TestCollector<T> {
        fn new() -> Self {
            Self {
                elements: Vec::new(),
            }
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

    #[test]
    fn test_window_operator_basic() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowOperator::new(Box::new(assigner), SumAggregator);
        let mut collector = TestCollector::<i64>::new();

        // Send records in window [0, 10_000)
        let r1 = Record::new(10i64, EventTimestamp::new(1_000)).with_key(vec![1]);
        let r2 = Record::new(20i64, EventTimestamp::new(5_000)).with_key(vec![1]);
        let r3 = Record::new(30i64, EventTimestamp::new(8_000)).with_key(vec![1]);

        op.process_record(r1, &mut collector).unwrap();
        op.process_record(r2, &mut collector).unwrap();
        op.process_record(r3, &mut collector).unwrap();

        // No output yet — watermark hasn't advanced past window end.
        assert!(collector.elements.is_empty());

        // Advance watermark past window end.
        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        // Should have emitted a result record and the watermark.
        let records: Vec<_> = collector
            .elements
            .iter()
            .filter_map(|e| match e {
                StreamElement::Record(r) => Some(r.value),
                _ => None,
            })
            .collect();
        assert_eq!(records, vec![60]); // 10 + 20 + 30
    }

    #[test]
    fn test_window_operator_multiple_keys() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowOperator::new(Box::new(assigner), SumAggregator);
        let mut collector = TestCollector::<i64>::new();

        // Key A
        op.process_record(
            Record::new(100i64, EventTimestamp::new(2_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();

        // Key B
        op.process_record(
            Record::new(200i64, EventTimestamp::new(3_000)).with_key(vec![2]),
            &mut collector,
        )
        .unwrap();

        // Fire
        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        let mut results: Vec<_> = collector
            .elements
            .iter()
            .filter_map(|e| match e {
                StreamElement::Record(r) => Some(r.value),
                _ => None,
            })
            .collect();
        results.sort();
        assert_eq!(results, vec![100, 200]);
    }

    #[test]
    fn test_window_operator_purges_after_fire() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowOperator::new(Box::new(assigner), SumAggregator);
        let mut collector = TestCollector::<i64>::new();

        op.process_record(
            Record::new(42i64, EventTimestamp::new(1_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();

        // First watermark fires and purges.
        let wm1 = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm1, &mut collector).unwrap();

        // Second watermark — accumulator was purged, nothing to fire.
        let wm2 = Watermark::new(EventTimestamp::new(20_000));
        op.process_watermark(wm2, &mut collector).unwrap();

        let record_count = collector
            .elements
            .iter()
            .filter(|e| matches!(e, StreamElement::Record(_)))
            .count();
        assert_eq!(record_count, 1); // Only one result from the first fire.
    }
}
