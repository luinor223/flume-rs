//! Integration tests for connected streams, CoProcessFunction, and side outputs.

use flume_api::{CollectSink, StreamExecutionEnvironment, TimestampedSource};
use flume_core::{
    CheckpointBarrier, CoProcessFunction, Collector, EventTimestamp, FlumeResult, OnTimerContext,
    OutputTag, ProcessContext, ProcessFunction, Record, StreamElement, Watermark,
};
use flume_runtime::side_output::SideOutputCollectors;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// CoProcessFunction implementations for testing
// ---------------------------------------------------------------------------

/// Tags outputs with which input they came from.
struct TaggingCoProcess;

impl CoProcessFunction<i32, String, String> for TaggingCoProcess {
    fn process_element1(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<String>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(format!("left:{value}"), ctx.timestamp()))
    }

    fn process_element2(
        &mut self,
        value: String,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<String>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(format!("right:{value}"), ctx.timestamp()))
    }
}

/// CoProcessFunction that accumulates from both inputs into shared state.
/// element1 adds to a counter, element2 reads the counter.
/// Output format: "input1:count" or "input2:count".
struct SharedStateCoProcess {
    counter: i32,
}

impl SharedStateCoProcess {
    fn new() -> Self {
        Self { counter: 0 }
    }
}

impl CoProcessFunction<i32, i32, String> for SharedStateCoProcess {
    fn process_element1(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<String>,
    ) -> FlumeResult<()> {
        self.counter += value;
        collector.collect(Record::new(
            format!("input1:{}", self.counter),
            ctx.timestamp(),
        ))
    }

    fn process_element2(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<String>,
    ) -> FlumeResult<()> {
        self.counter += value;
        collector.collect(Record::new(
            format!("input2:{}", self.counter),
            ctx.timestamp(),
        ))
    }
}

/// CoProcessFunction that registers a timer in process_element1.
struct TimerCoProcess;

impl CoProcessFunction<i32, i32, i32> for TimerCoProcess {
    fn process_element1(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        let timer_time = EventTimestamp::new(ctx.timestamp().as_millis() + 100);
        ctx.timer_service().register_event_time_timer(timer_time);
        collector.collect(Record::new(value, ctx.timestamp()))
    }

    fn process_element2(
        &mut self,
        value: i32,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<i32>,
    ) -> FlumeResult<()> {
        collector.collect(Record::new(value * 10, ctx.timestamp()))
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_basic_co_process_function() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    let elements1 = vec![
        StreamElement::Record(Record::new(1, EventTimestamp::new(1000))),
        StreamElement::Record(Record::new(2, EventTimestamp::new(2000))),
    ];

    let source2 = TimestampedSource::new(vec![
        StreamElement::Record(Record::new("hello".to_string(), EventTimestamp::new(1500))),
        StreamElement::Record(Record::new("world".to_string(), EventTimestamp::new(2500))),
    ]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .process(TaggingCoProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // All 4 outputs should appear. Order depends on the merger but all should be present.
    assert_eq!(values.len(), 4);
    let lefts: Vec<_> = values.iter().filter(|v| v.starts_with("left:")).collect();
    let rights: Vec<_> = values.iter().filter(|v| v.starts_with("right:")).collect();
    assert_eq!(lefts.len(), 2);
    assert_eq!(rights.len(), 2);
}

#[tokio::test]
async fn test_co_process_with_shared_state() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    // Source1: adds values to a shared counter.
    let elements1 = vec![
        StreamElement::Record(Record::new(10, EventTimestamp::new(100))),
        StreamElement::Record(Record::new(20, EventTimestamp::new(200))),
    ];

    // Source2: also adds values to the shared counter.
    let source2 = TimestampedSource::new(vec![StreamElement::Record(Record::new(
        5,
        EventTimestamp::new(150),
    ))]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .process(SharedStateCoProcess::new())
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // All 3 records should produce output; shared state means the counter
    // accumulates across both inputs. The exact order is non-deterministic
    // (depends on the merger), so we check: all 3 outputs exist, and the
    // final counter value (sum of 10 + 20 + 5 = 35) appears in the last output.
    assert_eq!(values.len(), 3);
    let input1_count: usize = values.iter().filter(|v| v.starts_with("input1:")).count();
    let input2_count: usize = values.iter().filter(|v| v.starts_with("input2:")).count();
    assert_eq!(input1_count, 2);
    assert_eq!(input2_count, 1);

    // The last output should have counter = 35 (regardless of order).
    let last = values.last().unwrap();
    let counter: i32 = last.split(':').next_back().unwrap().parse().unwrap();
    assert_eq!(counter, 35);
}

#[tokio::test]
async fn test_connected_stream_with_key_by_both() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    let elements1 = vec![StreamElement::Record(Record::new(
        42,
        EventTimestamp::new(1000),
    ))];

    let source2 = TimestampedSource::new(vec![StreamElement::Record(Record::new(
        "hello".to_string(),
        EventTimestamp::new(1500),
    ))]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .key_by_both(|v: &i32| *v, |s: &String| s.len())
        .process(TaggingCoProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(values.len(), 2);
    // Both outputs should be present.
    let lefts: Vec<_> = values.iter().filter(|v| v.starts_with("left:")).collect();
    let rights: Vec<_> = values.iter().filter(|v| v.starts_with("right:")).collect();
    assert_eq!(lefts.len(), 1);
    assert_eq!(rights.len(), 1);
}

#[tokio::test]
async fn test_co_process_with_timers() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Source1: one record at ts=1000 (registers timer at 1100), then watermark past 1100.
    let elements1 = vec![
        StreamElement::Record(Record::new(5, EventTimestamp::new(1000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1200))),
    ];

    // Source2: one record, then watermark past 1100 so min(wm1, wm2) >= 1100.
    let source2 = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(3, EventTimestamp::new(1050))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1200))),
    ]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .process(TimerCoProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Should have: 5 (from input1), 30 (3*10 from input2), -1 (timer fire).
    assert!(values.contains(&5), "input1 record should appear");
    assert!(values.contains(&30), "input2 record (3*10) should appear");
    assert!(values.contains(&-1), "timer fire should appear");
}

#[tokio::test]
async fn test_barrier_alignment_in_connected_stream() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    let barrier = CheckpointBarrier::new(1, EventTimestamp::new(500));

    // Both sources send records and barriers.
    let elements1 = vec![
        StreamElement::Record(Record::new(1, EventTimestamp::new(100))),
        StreamElement::CheckpointBarrier(barrier),
        StreamElement::Record(Record::new(2, EventTimestamp::new(600))),
    ];

    let source2 = TimestampedSource::new(vec![
        StreamElement::Record(Record::new("a".to_string(), EventTimestamp::new(200))),
        StreamElement::CheckpointBarrier(barrier),
        StreamElement::Record(Record::new("b".to_string(), EventTimestamp::new(700))),
    ]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .process(TaggingCoProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // All 4 records should appear (barriers are handled by the merger and
    // forwarded to the operator, which passes them downstream).
    assert_eq!(values.len(), 4);
}

#[tokio::test]
async fn test_side_output_emission() {
    // Test that SideOutputCollectors correctly emits to a side output channel
    // when used by a ProcessFunction through ProcessContext.
    let (side_tx, mut side_rx) = mpsc::channel::<StreamElement<String>>(16);

    let mut collectors = SideOutputCollectors::new();
    collectors.register("rejected".to_string(), side_tx);

    struct SideOutputProcess {
        tag: OutputTag<String>,
    }

    impl ProcessFunction<i32, i32> for SideOutputProcess {
        fn process_element(
            &mut self,
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            if value < 0 {
                ctx.side_output(&self.tag, format!("rejected:{value}"))?;
            } else {
                collector.collect(Record::new(value, ctx.timestamp()))?;
            }
            Ok(())
        }
    }

    // Use ProcessOperator with side outputs wired up.
    use flume_core::ProcessOperator;

    let process_fn = SideOutputProcess {
        tag: OutputTag::new("rejected"),
    };
    let mut op = ProcessOperator::new(process_fn).with_side_outputs(Box::new(collectors));

    // Create a simple test collector.
    use flume_core::Operator;
    let mut output: Vec<Record<i32>> = Vec::new();

    struct VecCollector<'a, T> {
        records: &'a mut Vec<Record<T>>,
    }

    impl<T: Send> Collector<T> for VecCollector<'_, T> {
        fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
            self.records.push(record);
            Ok(())
        }
        fn collect_watermark(&mut self, _: Watermark) -> FlumeResult<()> {
            Ok(())
        }
        fn collect_barrier(&mut self, _: CheckpointBarrier) -> FlumeResult<()> {
            Ok(())
        }
    }

    let mut collector = VecCollector {
        records: &mut output,
    };

    // Process positive value → main output.
    op.process_record(Record::new(42, EventTimestamp::new(100)), &mut collector)
        .unwrap();
    // Process negative value → side output.
    op.process_record(Record::new(-5, EventTimestamp::new(200)), &mut collector)
        .unwrap();

    // Main output should have only the positive value.
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].value, 42);

    // Side output should have the rejected record.
    let side_element = side_rx.recv().await.unwrap();
    if let StreamElement::Record(record) = side_element {
        assert_eq!(record.value, "rejected:-5");
        assert_eq!(record.timestamp, EventTimestamp::new(200));
    } else {
        panic!("expected Record on side output");
    }
}

#[tokio::test]
async fn test_one_input_finishes_before_the_other() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    // Source1: finishes immediately (no records).
    let elements1: Vec<StreamElement<i32>> = vec![];

    // Source2: has records.
    let source2 = TimestampedSource::new(vec![
        StreamElement::Record(Record::new("a".to_string(), EventTimestamp::new(100))),
        StreamElement::Record(Record::new("b".to_string(), EventTimestamp::new(200))),
    ]);

    env.from_source(TimestampedSource::new(elements1))
        .connect(source2)
        .process(TaggingCoProcess)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Only source2 records should appear.
    assert_eq!(values.len(), 2);
    assert!(values.iter().all(|v| v.starts_with("right:")));
}
