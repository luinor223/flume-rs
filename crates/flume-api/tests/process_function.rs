//! Integration tests for process functions.

use flume_api::{CollectSink, StreamExecutionEnvironment, TimestampedSource};
use flume_core::{
    Collector, EventTimestamp, FlumeResult, KeyedProcessFunction, OnTimerContext, ProcessContext,
    ProcessFunction, Record, StreamElement, Watermark,
};

// ---------------------------------------------------------------------------
// Process function implementations for testing
// ---------------------------------------------------------------------------

/// Doubles each value and emits via collector.
struct DoublingProcess;

impl ProcessFunction<i32, i32> for DoublingProcess {
    fn process_element(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(value * 2, ctx.timestamp()))
    }
}

/// Filtering process function: emits only even values, and for those emits
/// both the value and its square.
struct FilterAndExpandProcess;

impl ProcessFunction<i32, i32> for FilterAndExpandProcess {
    fn process_element(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        if value % 2 == 0 {
            collector.collect(Record::new(value, ctx.timestamp()))?;
            collector.collect(Record::new(value * value, ctx.timestamp()))?;
        }
        Ok(())
    }
}

/// Process function that registers an event-time timer 100ms in the future.
/// On timer fire, emits a sentinel value of -1.
struct TimerProcess;

impl ProcessFunction<i32, i32> for TimerProcess {
    fn process_element(
        &mut self,
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
        _ctx: &mut OnTimerContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(-1, timestamp))
    }
}

/// Keyed process function that adds key length to the value.
struct KeyLengthProcess;

impl KeyedProcessFunction<i32, i32> for KeyLengthProcess {
    fn process_element(
        &mut self,
        key: &[u8],
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(value + key.len() as i32, ctx.timestamp()))
    }
}

/// Keyed process function with per-key timers.
/// On element: registers timer at timestamp + 100.
/// On timer: emits the first byte of the key as value.
struct KeyedTimerProcess;

impl KeyedProcessFunction<i32, i32> for KeyedTimerProcess {
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
        let val = key.first().copied().unwrap_or(0) as i32;
        collector.collect(Record::new(val, timestamp))
    }
}

/// Process function that registers then deletes a timer.
struct TimerDeleteProcess;

impl ProcessFunction<i32, i32> for TimerDeleteProcess {
    fn process_element(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        let timer_time = EventTimestamp::new(ctx.timestamp().as_millis() + 100);
        ctx.timer_service().register_event_time_timer(timer_time);
        // Immediately delete the timer.
        ctx.timer_service().delete_event_time_timer(timer_time);
        collector.collect(Record::new(value, ctx.timestamp()))
    }

    fn on_timer(
        &mut self,
        timestamp: EventTimestamp,
        _ctx: &mut OnTimerContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        // Should never be called if deletion works.
        collector.collect(Record::new(-999, timestamp))
    }
}

/// Process function that registers the same timer twice.
struct TimerDedupProcess {
    timer_fire_count: i32,
}

impl TimerDedupProcess {
    fn new() -> Self {
        Self {
            timer_fire_count: 0,
        }
    }
}

impl ProcessFunction<i32, i32> for TimerDedupProcess {
    fn process_element(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        let timer_time = EventTimestamp::new(1100);
        // Register the same timer twice.
        ctx.timer_service().register_event_time_timer(timer_time);
        ctx.timer_service().register_event_time_timer(timer_time);
        collector.collect(Record::new(value, ctx.timestamp()))
    }

    fn on_timer(
        &mut self,
        timestamp: EventTimestamp,
        _ctx: &mut OnTimerContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        self.timer_fire_count += 1;
        collector.collect(Record::new(self.timer_fire_count, timestamp))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_basic_process_function() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        StreamElement::Record(Record::new(5, EventTimestamp::new(1000))),
        StreamElement::Record(Record::new(10, EventTimestamp::new(2000))),
        StreamElement::Record(Record::new(15, EventTimestamp::new(3000))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process(DoublingProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(*values, vec![10, 20, 30]);
}

#[tokio::test]
async fn test_filtering_process_function() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        StreamElement::Record(Record::new(1, EventTimestamp::new(1000))),
        StreamElement::Record(Record::new(2, EventTimestamp::new(2000))),
        StreamElement::Record(Record::new(3, EventTimestamp::new(3000))),
        StreamElement::Record(Record::new(4, EventTimestamp::new(4000))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process(FilterAndExpandProcess)
        .add_sink(sink)
        .await
        .unwrap();

    // Only even values pass: 2 -> (2, 4), 4 -> (4, 16)
    let values = results.lock().unwrap();
    assert_eq!(*values, vec![2, 4, 4, 16]);
}

#[tokio::test]
async fn test_process_function_with_event_time_timer() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        // Record at ts=1000 registers timer at 1100.
        StreamElement::Record(Record::new(42, EventTimestamp::new(1000))),
        // Watermark at 1050 — timer should NOT fire.
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1050))),
        // Watermark at 1100 — timer fires, emitting -1.
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1100))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process(TimerProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(*values, vec![42, -1]);
}

#[tokio::test]
async fn test_keyed_process_function() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        StreamElement::Record(Record::new(10, EventTimestamp::new(1000)).with_key(vec![1, 2, 3])),
        StreamElement::Record(Record::new(20, EventTimestamp::new(2000)).with_key(vec![4, 5])),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process_keyed(KeyLengthProcess)
        .add_sink(sink)
        .await
        .unwrap();

    // 10 + key.len(3) = 13, 20 + key.len(2) = 22
    let values = results.lock().unwrap();
    assert_eq!(*values, vec![13, 22]);
}

#[tokio::test]
async fn test_keyed_process_function_with_per_key_timers() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        // Key [1] at ts=1000 registers timer at 1100.
        StreamElement::Record(Record::new(10, EventTimestamp::new(1000)).with_key(vec![1])),
        // Key [2] at ts=1000 registers timer at 1100.
        StreamElement::Record(Record::new(20, EventTimestamp::new(1000)).with_key(vec![2])),
        // Watermark fires both timers.
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1100))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process_keyed(KeyedTimerProcess)
        .add_sink(sink)
        .await
        .unwrap();

    // Original values: 10, 20. Timer fires: key[1]->1, key[2]->2.
    let values = results.lock().unwrap();
    assert_eq!(*values, vec![10, 20, 1, 2]);
}

#[tokio::test]
async fn test_timer_deletion() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        // Record registers then immediately deletes a timer.
        StreamElement::Record(Record::new(42, EventTimestamp::new(1000))),
        // Watermark past the would-be timer time — should NOT fire.
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1200))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process(TimerDeleteProcess)
        .add_sink(sink)
        .await
        .unwrap();

    // Only the original record — no timer fire.
    let values = results.lock().unwrap();
    assert_eq!(*values, vec![42]);
}

#[tokio::test]
async fn test_timer_deduplication() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    let elements = vec![
        // Record registers the same timer (ts=1100) twice.
        StreamElement::Record(Record::new(42, EventTimestamp::new(1000))),
        // Watermark fires the timer — should fire only once.
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1200))),
    ];

    env.from_source(TimestampedSource::new(elements))
        .process(TimerDedupProcess::new())
        .add_sink(sink)
        .await
        .unwrap();

    // Original value 42, then exactly one timer fire (value = 1, the fire_count).
    let values = results.lock().unwrap();
    assert_eq!(*values, vec![42, 1]);
}
