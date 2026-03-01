//! Window join operators that match records from two streams within the same window.

use std::collections::HashMap;

use crate::either::Either;
use crate::join::{CogroupFunction, JoinFunction, emit_results};
use crate::window::{EventTimeTrigger, Trigger, TriggerResult, WindowAssigner};
use crate::{
    CheckpointBarrier, Collector, EventTimestamp, FlumeResult, Operator, Record, Watermark, Window,
};

/// Key for the per-key, per-window state map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WindowKey {
    key: Vec<u8>,
    window: Window,
}

/// Maximum number of key buffers to keep in the pool.
const KEY_POOL_CAP: usize = 1024;

/// Buffered records for one side of a window join.
struct WindowJoinState<In1, In2> {
    left: Vec<(In1, EventTimestamp)>,
    right: Vec<(In2, EventTimestamp)>,
}

impl<In1, In2> Default for WindowJoinState<In1, In2> {
    fn default() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
        }
    }
}

/// Window join operator: emits the cross product of matching left and right
/// records within each (key, window) pair when the window fires.
///
/// Implements `Operator<Either<In1, In2>, Out>`.
pub struct WindowJoinOperator<In1, In2, JF> {
    assigner: Box<dyn WindowAssigner>,
    trigger: Box<dyn Trigger>,
    join_fn: JF,
    state: HashMap<WindowKey, WindowJoinState<In1, In2>>,
    key_pool: Vec<Vec<u8>>,
}

impl<In1, In2, JF> WindowJoinOperator<In1, In2, JF> {
    pub fn new(assigner: Box<dyn WindowAssigner>, join_fn: JF) -> Self {
        Self {
            assigner,
            trigger: Box::new(EventTimeTrigger),
            join_fn,
            state: HashMap::new(),
            key_pool: Vec::new(),
        }
    }

    fn pool_get(&mut self) -> Vec<u8> {
        self.key_pool
            .pop()
            .map(|mut v| {
                v.clear();
                v
            })
            .unwrap_or_default()
    }

    fn pool_put(&mut self, buf: Vec<u8>) {
        if self.key_pool.len() < KEY_POOL_CAP {
            self.key_pool.push(buf);
        }
    }
}

impl<In1, In2, Out, JF> Operator<Either<In1, In2>, Out> for WindowJoinOperator<In1, In2, JF>
where
    In1: Send + Clone,
    In2: Send + Clone,
    Out: Send,
    JF: JoinFunction<In1, In2, Out>,
{
    fn process_record(
        &mut self,
        record: Record<Either<In1, In2>>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_else(|| self.pool_get());
        let timestamp = record.timestamp;
        let windows = self.assigner.assign_windows(timestamp);

        match record.value {
            Either::Left(value) => {
                for window in windows {
                    let wk = WindowKey {
                        key: key.clone(),
                        window,
                    };
                    self.state
                        .entry(wk)
                        .or_default()
                        .left
                        .push((value.clone(), timestamp));
                }
            }
            Either::Right(value) => {
                for window in windows {
                    let wk = WindowKey {
                        key: key.clone(),
                        window,
                    };
                    self.state
                        .entry(wk)
                        .or_default()
                        .right
                        .push((value.clone(), timestamp));
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
        let mut to_fire = Vec::new();
        let mut to_purge = Vec::new();

        for wk in self.state.keys() {
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

        // Fire: emit cross product of left x right.
        for wk in &to_fire {
            if let Some(join_state) = self.state.get(wk) {
                for (left_val, left_ts) in &join_state.left {
                    for (right_val, right_ts) in &join_state.right {
                        let output = self.join_fn.join(left_val, right_val);
                        let timestamp = (*left_ts).max(*right_ts);
                        let record = Record::new(output, timestamp).with_key(wk.key.clone());
                        collector.collect(record)?;
                    }
                }
            }
        }

        // Purge: remove state and return key buffers to the pool.
        for wk in to_purge {
            if self.state.remove(&wk).is_some() {
                self.pool_put(wk.key);
            }
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

/// Buffered records for one side of a cogroup.
struct CogroupState<In1, In2> {
    left: Vec<In1>,
    right: Vec<In2>,
}

impl<In1, In2> Default for CogroupState<In1, In2> {
    fn default() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
        }
    }
}

/// Window cogroup operator: collects all left and right records per (key, window)
/// and passes them as slices to a [`CogroupFunction`].
///
/// Implements `Operator<Either<In1, In2>, Out>`.
pub struct WindowCogroupOperator<In1, In2, CF> {
    assigner: Box<dyn WindowAssigner>,
    trigger: Box<dyn Trigger>,
    cogroup_fn: CF,
    state: HashMap<WindowKey, CogroupState<In1, In2>>,
    key_pool: Vec<Vec<u8>>,
}

impl<In1, In2, CF> WindowCogroupOperator<In1, In2, CF> {
    pub fn new(assigner: Box<dyn WindowAssigner>, cogroup_fn: CF) -> Self {
        Self {
            assigner,
            trigger: Box::new(EventTimeTrigger),
            cogroup_fn,
            state: HashMap::new(),
            key_pool: Vec::new(),
        }
    }

    fn pool_get(&mut self) -> Vec<u8> {
        self.key_pool
            .pop()
            .map(|mut v| {
                v.clear();
                v
            })
            .unwrap_or_default()
    }

    fn pool_put(&mut self, buf: Vec<u8>) {
        if self.key_pool.len() < KEY_POOL_CAP {
            self.key_pool.push(buf);
        }
    }
}

impl<In1, In2, Out, CF> Operator<Either<In1, In2>, Out> for WindowCogroupOperator<In1, In2, CF>
where
    In1: Send + Clone,
    In2: Send + Clone,
    Out: Send,
    CF: CogroupFunction<In1, In2, Out>,
{
    fn process_record(
        &mut self,
        record: Record<Either<In1, In2>>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_else(|| self.pool_get());
        let timestamp = record.timestamp;
        let windows = self.assigner.assign_windows(timestamp);

        match record.value {
            Either::Left(value) => {
                for window in windows {
                    let wk = WindowKey {
                        key: key.clone(),
                        window,
                    };
                    self.state.entry(wk).or_default().left.push(value.clone());
                }
            }
            Either::Right(value) => {
                for window in windows {
                    let wk = WindowKey {
                        key: key.clone(),
                        window,
                    };
                    self.state.entry(wk).or_default().right.push(value.clone());
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
        let mut to_fire = Vec::new();
        let mut to_purge = Vec::new();

        for wk in self.state.keys() {
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

        // Fire: call cogroup function and emit each result.
        for wk in &to_fire {
            if let Some(cg_state) = self.state.get(wk) {
                let results = self
                    .cogroup_fn
                    .cogroup(&wk.key, &cg_state.left, &cg_state.right);
                let timestamp = EventTimestamp::new(wk.window.end.as_millis() - 1);
                emit_results(results, wk.key.clone(), timestamp, collector)?;
            }
        }

        // Purge.
        for wk in to_purge {
            if self.state.remove(&wk).is_some() {
                self.pool_put(wk.key);
            }
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
    use std::time::Duration;

    use super::*;
    use crate::window::TumblingWindow;
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
    fn test_window_join_basic() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowJoinOperator::new(Box::new(assigner), |l: &i32, r: &i32| l + r);
        let mut collector = TestCollector::new();

        // 2 left records in window [0, 10_000)
        let l1 = Record::new(Either::Left(1i32), EventTimestamp::new(1_000)).with_key(vec![1]);
        let l2 = Record::new(Either::Left(2i32), EventTimestamp::new(2_000)).with_key(vec![1]);

        // 3 right records in same window
        let r1 = Record::new(Either::Right(10i32), EventTimestamp::new(3_000)).with_key(vec![1]);
        let r2 = Record::new(Either::Right(20i32), EventTimestamp::new(4_000)).with_key(vec![1]);
        let r3 = Record::new(Either::Right(30i32), EventTimestamp::new(5_000)).with_key(vec![1]);

        op.process_record(l1, &mut collector).unwrap();
        op.process_record(l2, &mut collector).unwrap();
        op.process_record(r1, &mut collector).unwrap();
        op.process_record(r2, &mut collector).unwrap();
        op.process_record(r3, &mut collector).unwrap();

        // No output yet.
        assert!(collector.records().is_empty());

        // Fire the window.
        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        // 2 left x 3 right = 6 outputs.
        let records = collector.records();
        assert_eq!(records.len(), 6);

        let mut values: Vec<i32> = records.iter().map(|r| r.value).collect();
        values.sort();
        // 1+10=11, 1+20=21, 1+30=31, 2+10=12, 2+20=22, 2+30=32
        assert_eq!(values, vec![11, 12, 21, 22, 31, 32]);
    }

    #[test]
    fn test_window_join_no_match() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowJoinOperator::new(Box::new(assigner), |l: &i32, r: &i32| l + r);
        let mut collector = TestCollector::new();

        // Only left records, no right records (inner join → 0 outputs).
        let l1 = Record::new(Either::Left(1i32), EventTimestamp::new(1_000)).with_key(vec![1]);
        let l2 = Record::new(Either::Left(2i32), EventTimestamp::new(2_000)).with_key(vec![1]);

        op.process_record(l1, &mut collector).unwrap();
        op.process_record(l2, &mut collector).unwrap();

        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        assert!(collector.records().is_empty());
    }

    #[test]
    fn test_window_join_multiple_keys() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowJoinOperator::new(Box::new(assigner), |l: &i32, r: &i32| l * r);
        let mut collector = TestCollector::new();

        // Key A: 1 left x 1 right
        let la = Record::new(Either::Left(2i32), EventTimestamp::new(1_000)).with_key(vec![1]);
        let ra = Record::new(Either::Right(3i32), EventTimestamp::new(2_000)).with_key(vec![1]);

        // Key B: 1 left x 1 right
        let lb = Record::new(Either::Left(5i32), EventTimestamp::new(3_000)).with_key(vec![2]);
        let rb = Record::new(Either::Right(7i32), EventTimestamp::new(4_000)).with_key(vec![2]);

        op.process_record(la, &mut collector).unwrap();
        op.process_record(ra, &mut collector).unwrap();
        op.process_record(lb, &mut collector).unwrap();
        op.process_record(rb, &mut collector).unwrap();

        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 2);

        let mut values: Vec<i32> = records.iter().map(|r| r.value).collect();
        values.sort();
        assert_eq!(values, vec![6, 35]); // 2*3=6, 5*7=35
    }

    #[test]
    fn test_window_join_output_timestamp() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowJoinOperator::new(Box::new(assigner), |l: &i32, r: &i32| l + r);
        let mut collector = TestCollector::new();

        let l1 =
            Record::new(Either::Left(1i32), EventTimestamp::new(2_000)).with_key(vec![1]);
        let r1 =
            Record::new(Either::Right(10i32), EventTimestamp::new(7_000)).with_key(vec![1]);

        op.process_record(l1, &mut collector).unwrap();
        op.process_record(r1, &mut collector).unwrap();

        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        // Output timestamp = max(2000, 7000) = 7000
        assert_eq!(records[0].timestamp, EventTimestamp::new(7_000));
    }

    #[test]
    fn test_window_join_purges_after_fire() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowJoinOperator::new(Box::new(assigner), |l: &i32, r: &i32| l + r);
        let mut collector = TestCollector::new();

        let l1 = Record::new(Either::Left(1i32), EventTimestamp::new(1_000)).with_key(vec![1]);
        let r1 = Record::new(Either::Right(10i32), EventTimestamp::new(2_000)).with_key(vec![1]);

        op.process_record(l1, &mut collector).unwrap();
        op.process_record(r1, &mut collector).unwrap();

        // First watermark fires.
        let wm1 = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm1, &mut collector).unwrap();
        assert_eq!(collector.records().len(), 1);

        // Second watermark — state was purged, no duplicates.
        let wm2 = Watermark::new(EventTimestamp::new(20_000));
        op.process_watermark(wm2, &mut collector).unwrap();
        assert_eq!(collector.records().len(), 1);
    }

    #[test]
    fn test_window_cogroup_basic() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowCogroupOperator::new(
            Box::new(assigner),
            |_key: &[u8], left: &[i32], right: &[i32]| {
                vec![format!("left={},right={}", left.len(), right.len())]
            },
        );
        let mut collector = TestCollector::new();

        // 2 left + 3 right
        op.process_record(
            Record::new(Either::Left(1i32), EventTimestamp::new(1_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();
        op.process_record(
            Record::new(Either::Left(2i32), EventTimestamp::new(2_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();
        op.process_record(
            Record::new(Either::Right(10i32), EventTimestamp::new(3_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();
        op.process_record(
            Record::new(Either::Right(20i32), EventTimestamp::new(4_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();
        op.process_record(
            Record::new(Either::Right(30i32), EventTimestamp::new(5_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();

        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, "left=2,right=3");
        // Output timestamp = window.end - 1 = 9999
        assert_eq!(records[0].timestamp, EventTimestamp::new(9_999));
    }

    #[test]
    fn test_window_cogroup_empty_side() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        let mut op = WindowCogroupOperator::new(
            Box::new(assigner),
            |_key: &[u8], left: &[i32], right: &[i32]| {
                vec![format!("left={},right={}", left.len(), right.len())]
            },
        );
        let mut collector = TestCollector::new();

        // Only left records, no right.
        op.process_record(
            Record::new(Either::Left(1i32), EventTimestamp::new(1_000)).with_key(vec![1]),
            &mut collector,
        )
        .unwrap();

        let wm = Watermark::new(EventTimestamp::new(10_000));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        // Cogroup fires with empty right side.
        assert_eq!(records[0].value, "left=1,right=0");
    }
}
