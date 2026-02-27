//! Operator adapters that wrap process functions into the `Operator` trait.

use crate::process::KeyedProcessFunction;
use crate::timer::TimerServiceImpl;
use crate::{
    CheckpointBarrier, Collector, FlumeResult, OnTimerContext, Operator, ProcessContext,
    ProcessFunction, Record, Watermark,
};

/// Wraps a [`ProcessFunction`] into an [`Operator`], providing timer support.
pub struct ProcessOperator<PF> {
    process_fn: PF,
    timer_service: TimerServiceImpl,
}

impl<PF> ProcessOperator<PF> {
    pub fn new(process_fn: PF) -> Self {
        Self {
            process_fn,
            timer_service: TimerServiceImpl::new(),
        }
    }
}

impl<In, Out, PF> Operator<In, Out> for ProcessOperator<PF>
where
    In: Send,
    Out: Send,
    PF: ProcessFunction<In, Out>,
{
    fn process_record(
        &mut self,
        record: Record<In>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_default();
        self.timer_service.set_current_key(key.clone());

        let mut ctx = ProcessContext::new(
            record.timestamp,
            if record.key.is_some() {
                Some(&key)
            } else {
                None
            },
            &mut self.timer_service,
        );

        self.process_fn
            .process_element(record.value, &mut ctx, collector)
    }

    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        self.timer_service.set_watermark(watermark.timestamp);

        let fired = self
            .timer_service
            .queue_mut()
            .fire_event_time_up_to(watermark.timestamp);

        for (timestamp, key) in fired {
            self.timer_service.set_current_key(key);
            let mut ctx = OnTimerContext::new(watermark.timestamp, &mut self.timer_service);
            self.process_fn.on_timer(timestamp, &mut ctx, collector)?;
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

/// Wraps a [`KeyedProcessFunction`] into an [`Operator`], providing
/// per-key timer support. Expects records to have a key set (via `key_by`).
pub struct KeyedProcessOperator<PF> {
    process_fn: PF,
    timer_service: TimerServiceImpl,
}

impl<PF> KeyedProcessOperator<PF> {
    pub fn new(process_fn: PF) -> Self {
        Self {
            process_fn,
            timer_service: TimerServiceImpl::new(),
        }
    }
}

impl<In, Out, PF> Operator<In, Out> for KeyedProcessOperator<PF>
where
    In: Send,
    Out: Send,
    PF: KeyedProcessFunction<In, Out>,
{
    fn process_record(
        &mut self,
        record: Record<In>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let key = record.key.clone().unwrap_or_default();
        self.timer_service.set_current_key(key.clone());

        let mut ctx = ProcessContext::new(record.timestamp, Some(&key), &mut self.timer_service);

        self.process_fn
            .process_element(&key, record.value, &mut ctx, collector)
    }

    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        self.timer_service.set_watermark(watermark.timestamp);

        let fired = self
            .timer_service
            .queue_mut()
            .fire_event_time_up_to(watermark.timestamp);

        for (timestamp, key) in fired {
            self.timer_service.set_current_key(key.clone());
            let mut ctx = OnTimerContext::new(watermark.timestamp, &mut self.timer_service);
            self.process_fn
                .on_timer(timestamp, &key, &mut ctx, collector)?;
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
    use crate::{EventTimestamp, StreamElement};

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

    /// Simple doubling process function.
    struct DoublerPF;

    impl ProcessFunction<i32, i32> for DoublerPF {
        fn process_element(
            &mut self,
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            collector.collect(Record::new(value * 2, ctx.timestamp()))
        }
    }

    /// Process function that registers a timer on each element.
    struct TimerRegisterPF;

    impl ProcessFunction<i32, i32> for TimerRegisterPF {
        fn process_element(
            &mut self,
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            // Register a timer 100ms after the record timestamp.
            let timer_time = EventTimestamp::new(ctx.timestamp().as_millis() + 100);
            ctx.timer_service().register_event_time_timer(timer_time);
            collector.collect(Record::new(value, ctx.timestamp()))
        }

        fn on_timer(
            &mut self,
            timestamp: EventTimestamp,
            _ctx: &mut OnTimerContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            // Emit a sentinel value when the timer fires.
            collector.collect(Record::new(-1, timestamp))
        }
    }

    /// Keyed process function that echoes the key length.
    struct KeyEchoPF;

    impl KeyedProcessFunction<i32, i32> for KeyEchoPF {
        fn process_element(
            &mut self,
            key: &[u8],
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            collector.collect(
                Record::new(value + key.len() as i32, ctx.timestamp()).with_key(key.to_vec()),
            )
        }
    }

    /// Keyed process function with per-key timers.
    struct KeyedTimerPF;

    impl KeyedProcessFunction<i32, i32> for KeyedTimerPF {
        fn process_element(
            &mut self,
            _key: &[u8],
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            let timer_time = EventTimestamp::new(ctx.timestamp().as_millis() + 100);
            ctx.timer_service().register_event_time_timer(timer_time);
            collector.collect(Record::new(value, ctx.timestamp()))
        }

        fn on_timer(
            &mut self,
            timestamp: EventTimestamp,
            key: &[u8],
            _ctx: &mut OnTimerContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            // Emit the key's first byte as the value.
            let val = key.first().copied().unwrap_or(0) as i32;
            collector.collect(Record::new(val, timestamp).with_key(key.to_vec()))
        }
    }

    #[test]
    fn test_process_operator_forwards_records() {
        let mut op = ProcessOperator::new(DoublerPF);
        let mut collector = TestCollector::new();

        let record = Record::new(5, EventTimestamp::new(100));
        op.process_record(record, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 10);
        assert_eq!(records[0].timestamp, EventTimestamp::new(100));
    }

    #[test]
    fn test_process_operator_fires_timers_on_watermark() {
        let mut op = ProcessOperator::new(TimerRegisterPF);
        let mut collector = TestCollector::new();

        // Process a record at timestamp 1000 — registers timer at 1100.
        let record = Record::new(42, EventTimestamp::new(1000));
        op.process_record(record, &mut collector).unwrap();

        assert_eq!(collector.records().len(), 1);
        assert_eq!(collector.records()[0].value, 42);

        // Watermark at 1050 — timer at 1100 should not fire.
        let wm = Watermark::new(EventTimestamp::new(1050));
        op.process_watermark(wm, &mut collector).unwrap();
        assert_eq!(collector.records().len(), 1); // still just the original

        // Watermark at 1100 — timer fires, emits sentinel -1.
        let wm = Watermark::new(EventTimestamp::new(1100));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].value, -1);
        assert_eq!(records[1].timestamp, EventTimestamp::new(1100));
    }

    #[test]
    fn test_keyed_process_operator_routes_key() {
        let mut op = KeyedProcessOperator::new(KeyEchoPF);
        let mut collector = TestCollector::new();

        let record = Record::new(10, EventTimestamp::new(100)).with_key(vec![1, 2, 3]);
        op.process_record(record, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, 13); // 10 + key.len(3)
        assert_eq!(records[0].key, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_keyed_process_operator_fires_per_key_timers() {
        let mut op = KeyedProcessOperator::new(KeyedTimerPF);
        let mut collector = TestCollector::new();

        // Key A records at ts=1000, registers timer at 1100.
        let r1 = Record::new(10, EventTimestamp::new(1000)).with_key(vec![1]);
        op.process_record(r1, &mut collector).unwrap();

        // Key B records at ts=1000, registers timer at 1100.
        let r2 = Record::new(20, EventTimestamp::new(1000)).with_key(vec![2]);
        op.process_record(r2, &mut collector).unwrap();

        assert_eq!(collector.records().len(), 2);

        // Advance watermark past 1100 — both timers fire.
        let wm = Watermark::new(EventTimestamp::new(1100));
        op.process_watermark(wm, &mut collector).unwrap();

        let records = collector.records();
        assert_eq!(records.len(), 4); // 2 original + 2 timer fires

        // Timer fires: key [1] -> value 1, key [2] -> value 2.
        let timer_records: Vec<_> = records[2..].to_vec();
        assert_eq!(timer_records[0].value, 1);
        assert_eq!(timer_records[0].key, Some(vec![1]));
        assert_eq!(timer_records[1].value, 2);
        assert_eq!(timer_records[1].key, Some(vec![2]));
    }

    #[test]
    fn test_process_operator_forwards_watermark() {
        let mut op = ProcessOperator::new(DoublerPF);
        let mut collector = TestCollector::new();

        let wm = Watermark::new(EventTimestamp::new(500));
        op.process_watermark(wm, &mut collector).unwrap();

        assert!(matches!(
            collector.elements.last(),
            Some(StreamElement::Watermark(w)) if w.timestamp == EventTimestamp::new(500)
        ));
    }

    #[test]
    fn test_process_operator_forwards_barrier() {
        let mut op = ProcessOperator::new(DoublerPF);
        let mut collector = TestCollector::new();

        let barrier = CheckpointBarrier::new(1, EventTimestamp::new(100));
        op.process_barrier(barrier, &mut collector).unwrap();

        assert!(matches!(
            collector.elements.last(),
            Some(StreamElement::CheckpointBarrier(b)) if b.checkpoint_id == 1
        ));
    }
}
