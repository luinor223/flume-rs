//! Interval join operator that matches records from two streams based on
//! event-time proximity within configurable bounds.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use crate::either::Either;
use crate::join::JoinFunction;
use crate::{
    CheckpointBarrier, Collector, EventTimestamp, FlumeResult, Operator, Record, Watermark,
};

/// Per-key state for interval join: sorted buffers for each side.
struct IntervalJoinState<In1, In2> {
    left: BTreeMap<EventTimestamp, Vec<In1>>,
    right: BTreeMap<EventTimestamp, Vec<In2>>,
}

impl<In1, In2> Default for IntervalJoinState<In1, In2> {
    fn default() -> Self {
        Self {
            left: BTreeMap::new(),
            right: BTreeMap::new(),
        }
    }
}

/// Interval join operator: for each left record at `t_l`, matches all right
/// records at `t_r` where `t_l <= t_r <= t_l + upper_bound`, and vice versa.
///
/// Uses non-negative `Duration` bounds: a left record at `t_l` matches right
/// records in `[t_l, t_l + upper_bound]`, and a right record at `t_r` matches
/// left records in `[t_r - upper_bound, t_r - lower_bound]`.
///
/// Watermark-based eviction removes records that can never match future inputs.
pub struct IntervalJoinOperator<In1, In2, JF> {
    lower_bound: Duration,
    upper_bound: Duration,
    join_fn: JF,
    state: HashMap<Vec<u8>, IntervalJoinState<In1, In2>>,
    current_watermark: EventTimestamp,
}

impl<In1, In2, JF> IntervalJoinOperator<In1, In2, JF> {
    pub fn new(lower_bound: Duration, upper_bound: Duration, join_fn: JF) -> Self {
        Self {
            lower_bound,
            upper_bound,
            join_fn,
            state: HashMap::new(),
            current_watermark: EventTimestamp::MIN,
        }
    }
}

impl<In1, In2, Out, JF> Operator<Either<In1, In2>, Out> for IntervalJoinOperator<In1, In2, JF>
where
    In1: Send + Clone,
    In2: Send + Clone,
    Out: Send,
    JF: JoinFunction<In1, In2, Out>,
{
    fn process_record(
        &mut self,
        record: Record<Either<In1, In2>>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_default();
        let timestamp = record.timestamp;
        let per_key = self.state.entry(key.clone()).or_default();

        match record.value {
            Either::Left(value) => {
                // Store this left record.
                per_key
                    .left
                    .entry(timestamp)
                    .or_default()
                    .push(value.clone());

                // Find matching right records: t_r in [t_l + lower_bound, t_l + upper_bound]
                let range_start = timestamp + self.lower_bound;
                let range_end = timestamp + self.upper_bound;

                for (&right_ts, right_values) in per_key.right.range(range_start..=range_end) {
                    for right_val in right_values {
                        let output = self.join_fn.join(&value, right_val);
                        let out_ts = timestamp.max(right_ts);
                        collector.collect(Record::new(output, out_ts).with_key(key.clone()))?;
                    }
                }
            }
            Either::Right(value) => {
                // Store this right record.
                per_key
                    .right
                    .entry(timestamp)
                    .or_default()
                    .push(value.clone());

                // Find matching left records: t_l where t_r is in [t_l + lower, t_l + upper]
                // Rearranging: t_r - upper <= t_l <= t_r - lower
                let range_start = timestamp - self.upper_bound;
                let range_end = timestamp - self.lower_bound;

                for (&left_ts, left_values) in per_key.left.range(range_start..=range_end) {
                    for left_val in left_values {
                        let output = self.join_fn.join(left_val, &value);
                        let out_ts = left_ts.max(timestamp);
                        collector.collect(Record::new(output, out_ts).with_key(key.clone()))?;
                    }
                }
            }
        }

        Ok(())
    }

    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        self.current_watermark = watermark.timestamp;

        // Evict expired records from all keys.
        let mut empty_keys = Vec::new();

        for (key, per_key) in &mut self.state {
            // Left records: evict where t_l + upper_bound < watermark
            // (can never match a future right record at t_r >= watermark)
            let left_cutoff = watermark.timestamp - self.upper_bound;
            let to_remove_left: Vec<EventTimestamp> = per_key
                .left
                .range(..left_cutoff)
                .map(|(&ts, _)| ts)
                .collect();
            for ts in to_remove_left {
                per_key.left.remove(&ts);
            }

            // Right records: evict where t_r + upper_bound < watermark
            // (can never match a future left record at t_l >= watermark)
            let to_remove_right: Vec<EventTimestamp> = per_key
                .right
                .range(..left_cutoff)
                .map(|(&ts, _)| ts)
                .collect();
            for ts in to_remove_right {
                per_key.right.remove(&ts);
            }

            if per_key.left.is_empty() && per_key.right.is_empty() {
                empty_keys.push(key.clone());
            }
        }

        for key in empty_keys {
            self.state.remove(&key);
        }

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
    use super::*;
    use crate::StreamElement;

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

        fn records(&self) -> Vec<&Record<T>> {
            self.elements
                .iter()
                .filter_map(|e| match e {
                    StreamElement::Record(r) => Some(r),
                    _ => None,
                })
                .collect()
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
    fn test_interval_join_basic_match() {
        // between(0, 1s): left at 100 matches right at 150 (100+0 <= 150 <= 100+1000)
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_secs(1),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        let right = Record::new(Either::Right(10i32), EventTimestamp::new(150)).with_key(vec![1]);

        op.process_record(left, &mut collector).unwrap();
        op.process_record(right, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 11);
    }

    #[test]
    fn test_interval_join_no_match() {
        // between(0, 100ms): left at 100, right at 300 (outside 100+100=200)
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_millis(100),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        let right = Record::new(Either::Right(10i32), EventTimestamp::new(300)).with_key(vec![1]);

        op.process_record(left, &mut collector).unwrap();
        op.process_record(right, &mut collector).unwrap();

        assert!(collector.records().is_empty());
    }

    #[test]
    fn test_interval_join_multiple_matches() {
        // between(0, 1s): left at 100 matches rights at 200, 500, 800
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_secs(1),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        op.process_record(left, &mut collector).unwrap();

        for ts in [200, 500, 800] {
            let right =
                Record::new(Either::Right(10i32), EventTimestamp::new(ts)).with_key(vec![1]);
            op.process_record(right, &mut collector).unwrap();
        }

        assert_eq!(collector.records().len(), 3);
        assert!(collector.records().iter().all(|r| r.value == 11));
    }

    #[test]
    fn test_interval_join_bidirectional() {
        // Right arrives first, then left matches retroactively.
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_secs(1),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        // Right at t=500
        let right = Record::new(Either::Right(10i32), EventTimestamp::new(500)).with_key(vec![1]);
        op.process_record(right, &mut collector).unwrap();

        // No output yet (no left records to match).
        assert!(collector.records().is_empty());

        // Left at t=100, matches right at 500 because 100+0 <= 500 <= 100+1000.
        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        op.process_record(left, &mut collector).unwrap();

        assert_eq!(collector.records().len(), 1);
        assert_eq!(collector.records()[0].value, 11);
    }

    #[test]
    fn test_interval_join_watermark_eviction() {
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_millis(500),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        // Left at t=100
        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        op.process_record(left, &mut collector).unwrap();

        // Watermark at 700 evicts left record at 100 (100 + 500 < 700).
        let wm = Watermark::new(EventTimestamp::new(700));
        op.process_watermark(wm, &mut collector).unwrap();

        // Right at 200 would have matched left at 100, but it was evicted.
        let right = Record::new(Either::Right(10i32), EventTimestamp::new(200)).with_key(vec![1]);
        op.process_record(right, &mut collector).unwrap();

        assert!(collector.records().is_empty());
    }

    #[test]
    fn test_interval_join_multiple_keys() {
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_secs(1),
            |l: &i32, r: &i32| l * r,
        );
        let mut collector = TestCollector::new();

        // Key A
        let la = Record::new(Either::Left(2i32), EventTimestamp::new(100)).with_key(vec![1]);
        let ra = Record::new(Either::Right(3i32), EventTimestamp::new(200)).with_key(vec![1]);

        // Key B
        let lb = Record::new(Either::Left(5i32), EventTimestamp::new(100)).with_key(vec![2]);
        let rb = Record::new(Either::Right(7i32), EventTimestamp::new(200)).with_key(vec![2]);

        op.process_record(la, &mut collector).unwrap();
        op.process_record(lb, &mut collector).unwrap();
        op.process_record(ra, &mut collector).unwrap();
        op.process_record(rb, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 2);

        let mut values: Vec<i32> = records.iter().map(|r| r.value).collect();
        values.sort();
        assert_eq!(values, vec![6, 35]); // 2*3, 5*7

        // Verify keys are isolated.
        for record in &records {
            if record.value == 6 {
                assert_eq!(record.key, Some(vec![1]));
            } else {
                assert_eq!(record.key, Some(vec![2]));
            }
        }
    }

    #[test]
    fn test_interval_join_output_timestamp() {
        let mut op = IntervalJoinOperator::new(
            Duration::from_millis(0),
            Duration::from_secs(1),
            |l: &i32, r: &i32| l + r,
        );
        let mut collector = TestCollector::new();

        let left = Record::new(Either::Left(1i32), EventTimestamp::new(100)).with_key(vec![1]);
        let right = Record::new(Either::Right(10i32), EventTimestamp::new(500)).with_key(vec![1]);

        op.process_record(left, &mut collector).unwrap();
        op.process_record(right, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        // max(100, 500) = 500
        assert_eq!(records[0].timestamp, EventTimestamp::new(500));
    }
}
