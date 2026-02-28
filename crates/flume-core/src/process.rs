//! Process function traits for record-by-record processing with timers.
//!
//! Process functions are the most expressive API in flume-rs, giving users
//! access to per-record processing with event-time timers and the ability
//! to emit zero, one, or many output records per input.

use std::any::Any;
use std::marker::PhantomData;

use crate::{Collector, EventTimestamp, FlumeResult};

/// Trait for emitting records to side output channels.
///
/// Implemented in `flume-runtime` with concrete channel senders.
/// Defined here (in `flume-core`) so process functions can emit
/// side outputs without depending on tokio.
pub trait SideOutputEmitter: Send {
    /// Emit a type-erased value to the side output identified by `tag_id`.
    fn emit(
        &mut self,
        tag_id: &str,
        value: Box<dyn Any + Send>,
        timestamp: EventTimestamp,
        key: Option<Vec<u8>>,
    ) -> FlumeResult<()>;
}

/// Provides context about the current record being processed.
pub struct ProcessContext<'a> {
    timestamp: EventTimestamp,
    current_key: Option<&'a [u8]>,
    timer_service: &'a mut dyn TimerService,
    side_outputs: Option<&'a mut dyn SideOutputEmitter>,
}

impl<'a> ProcessContext<'a> {
    /// Create a new process context.
    pub fn new(
        timestamp: EventTimestamp,
        current_key: Option<&'a [u8]>,
        timer_service: &'a mut dyn TimerService,
    ) -> Self {
        Self {
            timestamp,
            current_key,
            timer_service,
            side_outputs: None,
        }
    }

    /// Create a new process context with side output support.
    pub fn with_side_outputs(
        timestamp: EventTimestamp,
        current_key: Option<&'a [u8]>,
        timer_service: &'a mut dyn TimerService,
        side_outputs: &'a mut dyn SideOutputEmitter,
    ) -> Self {
        Self {
            timestamp,
            current_key,
            timer_service,
            side_outputs: Some(side_outputs),
        }
    }

    /// The event-time timestamp of the current record.
    pub fn timestamp(&self) -> EventTimestamp {
        self.timestamp
    }

    /// The partition key of the current record, if any.
    pub fn current_key(&self) -> Option<&[u8]> {
        self.current_key
    }

    /// Access the timer service to register or delete timers.
    pub fn timer_service(&mut self) -> &mut dyn TimerService {
        self.timer_service
    }

    /// Emit a value to a side output channel identified by the given tag.
    ///
    /// Returns an error if no side output emitter is configured or if the
    /// tag is not registered.
    pub fn side_output<T: Send + 'static>(
        &mut self,
        tag: &OutputTag<T>,
        value: T,
    ) -> FlumeResult<()> {
        let emitter = self
            .side_outputs
            .as_deref_mut()
            .ok_or_else(|| crate::FlumeError::Operator("no side output emitter configured".into()))?;
        emitter.emit(
            tag.id(),
            Box::new(value),
            self.timestamp,
            self.current_key.map(|k| k.to_vec()),
        )
    }
}

/// Provides context when a timer fires.
pub struct OnTimerContext<'a> {
    current_watermark: EventTimestamp,
    timer_service: &'a mut dyn TimerService,
}

impl<'a> OnTimerContext<'a> {
    /// Create a new on-timer context.
    pub fn new(current_watermark: EventTimestamp, timer_service: &'a mut dyn TimerService) -> Self {
        Self {
            current_watermark,
            timer_service,
        }
    }

    /// The current watermark when the timer fired.
    pub fn current_watermark(&self) -> EventTimestamp {
        self.current_watermark
    }

    /// Access the timer service to register follow-up timers.
    pub fn timer_service(&mut self) -> &mut dyn TimerService {
        self.timer_service
    }
}

/// Service for registering and deleting event-time and processing-time timers.
pub trait TimerService {
    /// The current watermark.
    fn current_watermark(&self) -> EventTimestamp;

    /// Register an event-time timer that fires when watermark >= `time`.
    fn register_event_time_timer(&mut self, time: EventTimestamp);

    /// Delete a previously registered event-time timer.
    fn delete_event_time_timer(&mut self, time: EventTimestamp);

    /// Register a processing-time timer. Note: processing-time timers are
    /// not auto-fired in the synchronous hot path (no processing-time clock yet).
    fn register_processing_time_timer(&mut self, time: EventTimestamp);

    /// Delete a previously registered processing-time timer.
    fn delete_processing_time_timer(&mut self, time: EventTimestamp);

    /// Returns the current processing time (wall-clock).
    fn current_processing_time(&self) -> EventTimestamp;
}

/// A process function that processes records one at a time with access to
/// timers and the ability to emit arbitrary output via the collector.
///
/// This is the most flexible transformation API, suitable for complex
/// event-driven logic that cannot be expressed with simple map/filter/window.
pub trait ProcessFunction<In: Send, Out: Send>: Send {
    /// Process a single input value. May emit zero or more outputs.
    fn process_element(
        &mut self,
        value: In,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()>;

    /// Called when a previously registered timer fires.
    /// Default implementation does nothing.
    fn on_timer(
        &mut self,
        _timestamp: EventTimestamp,
        _ctx: &mut OnTimerContext<'_>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        Ok(())
    }
}

/// A keyed process function that receives the partition key alongside each
/// record. Used after `key_by()` for per-key stateful processing with timers.
pub trait KeyedProcessFunction<In: Send, Out: Send>: Send {
    /// Process a single input value with its partition key.
    fn process_element(
        &mut self,
        key: &[u8],
        value: In,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()>;

    /// Called when a previously registered timer fires, with the key it was
    /// registered for.
    fn on_timer(
        &mut self,
        _timestamp: EventTimestamp,
        _key: &[u8],
        _ctx: &mut OnTimerContext<'_>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        Ok(())
    }
}

/// A two-input process function for connected streams.
///
/// Receives elements from two input streams (dispatched via `Either<In1, In2>`)
/// and can emit outputs, register timers, and emit side outputs.
pub trait CoProcessFunction<In1: Send, In2: Send, Out: Send>: Send {
    /// Process an element from the first input stream.
    fn process_element1(
        &mut self,
        value: In1,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()>;

    /// Process an element from the second input stream.
    fn process_element2(
        &mut self,
        value: In2,
        ctx: &mut ProcessContext<'_>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()>;

    /// Called when a previously registered timer fires.
    /// Default implementation does nothing.
    fn on_timer(
        &mut self,
        _timestamp: EventTimestamp,
        _ctx: &mut OnTimerContext<'_>,
        _collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        Ok(())
    }
}

/// A named tag for a side output stream.
pub struct OutputTag<T> {
    id: String,
    _marker: PhantomData<T>,
}

impl<T> OutputTag<T> {
    /// Create a new output tag with the given identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            _marker: PhantomData,
        }
    }

    /// The identifier for this output tag.
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckpointBarrier, Record, Watermark};

    /// Mock timer service for testing.
    struct MockTimerService {
        watermark: EventTimestamp,
        event_timers: Vec<EventTimestamp>,
    }

    impl MockTimerService {
        fn new(watermark: EventTimestamp) -> Self {
            Self {
                watermark,
                event_timers: Vec::new(),
            }
        }
    }

    impl TimerService for MockTimerService {
        fn current_watermark(&self) -> EventTimestamp {
            self.watermark
        }

        fn register_event_time_timer(&mut self, time: EventTimestamp) {
            self.event_timers.push(time);
        }

        fn delete_event_time_timer(&mut self, time: EventTimestamp) {
            self.event_timers.retain(|t| *t != time);
        }

        fn register_processing_time_timer(&mut self, _time: EventTimestamp) {}

        fn delete_processing_time_timer(&mut self, _time: EventTimestamp) {}

        fn current_processing_time(&self) -> EventTimestamp {
            EventTimestamp::new(0)
        }
    }

    struct VecCollector<T> {
        records: Vec<Record<T>>,
    }

    impl<T> VecCollector<T> {
        fn new() -> Self {
            Self {
                records: Vec::new(),
            }
        }
    }

    impl<T: Send> Collector<T> for VecCollector<T> {
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

    /// A simple doubling process function.
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

    /// A keyed process function that prepends key length to value.
    struct KeyLengthProcess;

    impl KeyedProcessFunction<i32, i32> for KeyLengthProcess {
        fn process_element(
            &mut self,
            key: &[u8],
            value: i32,
            ctx: &mut ProcessContext<'_>,
            collector: &mut dyn Collector<i32>,
        ) -> FlumeResult<()> {
            let result = value + key.len() as i32;
            collector.collect(Record::new(result, ctx.timestamp()))
        }
    }

    #[test]
    fn test_process_function_trait() {
        let mut pf = DoublingProcess;
        let mut collector = VecCollector::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut ctx = ProcessContext::new(EventTimestamp::new(100), None, &mut timer_service);

        pf.process_element(5, &mut ctx, &mut collector).unwrap();

        assert_eq!(collector.records.len(), 1);
        assert_eq!(collector.records[0].value, 10);
        assert_eq!(collector.records[0].timestamp, EventTimestamp::new(100));
    }

    #[test]
    fn test_keyed_process_function_trait() {
        let mut pf = KeyLengthProcess;
        let mut collector = VecCollector::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let key = vec![1, 2, 3];
        let mut ctx = ProcessContext::new(EventTimestamp::new(200), Some(&key), &mut timer_service);

        pf.process_element(&key, 10, &mut ctx, &mut collector)
            .unwrap();

        assert_eq!(collector.records.len(), 1);
        assert_eq!(collector.records[0].value, 13); // 10 + key.len(3)
    }

    #[test]
    fn test_process_context_accessors() {
        let mut timer_service = MockTimerService::new(EventTimestamp::new(500));
        let key = vec![42];
        let mut ctx = ProcessContext::new(EventTimestamp::new(100), Some(&key), &mut timer_service);

        assert_eq!(ctx.timestamp(), EventTimestamp::new(100));
        assert_eq!(ctx.current_key(), Some(&[42u8][..]));
        assert_eq!(
            ctx.timer_service().current_watermark(),
            EventTimestamp::new(500)
        );
    }

    #[test]
    fn test_on_timer_context_accessors() {
        let mut timer_service = MockTimerService::new(EventTimestamp::new(1000));
        let mut ctx = OnTimerContext::new(EventTimestamp::new(1000), &mut timer_service);

        assert_eq!(ctx.current_watermark(), EventTimestamp::new(1000));
        assert_eq!(
            ctx.timer_service().current_watermark(),
            EventTimestamp::new(1000)
        );
    }

    #[test]
    fn test_timer_registration_via_context() {
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut ctx = ProcessContext::new(EventTimestamp::new(100), None, &mut timer_service);

        ctx.timer_service()
            .register_event_time_timer(EventTimestamp::new(500));

        // Verify the timer was registered in the mock — reborrow ends
        // when we stop using `ctx`.
        let _ = ctx.timestamp(); // keep borrow alive until here
        assert_eq!(timer_service.event_timers, vec![EventTimestamp::new(500)]);
    }

    #[test]
    fn test_output_tag() {
        let tag = OutputTag::<String>::new("side-output-1");
        assert_eq!(tag.id(), "side-output-1");
    }

    #[test]
    fn test_default_on_timer_is_noop() {
        let mut pf = DoublingProcess;
        let mut collector = VecCollector::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(500));
        let mut ctx = OnTimerContext::new(EventTimestamp::new(500), &mut timer_service);

        pf.on_timer(EventTimestamp::new(500), &mut ctx, &mut collector)
            .unwrap();

        assert!(collector.records.is_empty());
    }

    // -----------------------------------------------------------------------
    // CoProcessFunction tests
    // -----------------------------------------------------------------------

    /// A CoProcessFunction that tags outputs with which input they came from.
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

    #[test]
    fn test_co_process_function_element1() {
        let mut cpf = TaggingCoProcess;
        let mut collector = VecCollector::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut ctx = ProcessContext::new(EventTimestamp::new(100), None, &mut timer_service);

        cpf.process_element1(42, &mut ctx, &mut collector).unwrap();

        assert_eq!(collector.records.len(), 1);
        assert_eq!(collector.records[0].value, "left:42");
    }

    #[test]
    fn test_co_process_function_element2() {
        let mut cpf = TaggingCoProcess;
        let mut collector = VecCollector::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut ctx = ProcessContext::new(EventTimestamp::new(200), None, &mut timer_service);

        cpf.process_element2("hello".to_string(), &mut ctx, &mut collector)
            .unwrap();

        assert_eq!(collector.records.len(), 1);
        assert_eq!(collector.records[0].value, "right:hello");
    }

    #[test]
    fn test_co_process_function_default_on_timer() {
        let mut cpf = TaggingCoProcess;
        let mut collector = VecCollector::<String>::new();
        let mut timer_service = MockTimerService::new(EventTimestamp::new(500));
        let mut ctx = OnTimerContext::new(EventTimestamp::new(500), &mut timer_service);

        cpf.on_timer(EventTimestamp::new(500), &mut ctx, &mut collector)
            .unwrap();

        assert!(collector.records.is_empty());
    }

    // -----------------------------------------------------------------------
    // SideOutputEmitter tests
    // -----------------------------------------------------------------------

    /// Mock side output emitter that records emissions.
    struct MockSideOutputEmitter {
        emissions: Vec<(String, EventTimestamp)>,
    }

    impl MockSideOutputEmitter {
        fn new() -> Self {
            Self {
                emissions: Vec::new(),
            }
        }
    }

    impl SideOutputEmitter for MockSideOutputEmitter {
        fn emit(
            &mut self,
            tag_id: &str,
            _value: Box<dyn Any + Send>,
            timestamp: EventTimestamp,
            _key: Option<Vec<u8>>,
        ) -> FlumeResult<()> {
            self.emissions.push((tag_id.to_string(), timestamp));
            Ok(())
        }
    }

    #[test]
    fn test_side_output_via_context() {
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut emitter = MockSideOutputEmitter::new();

        {
            let mut ctx = ProcessContext::with_side_outputs(
                EventTimestamp::new(100),
                None,
                &mut timer_service,
                &mut emitter,
            );

            let tag = OutputTag::<String>::new("rejected");
            ctx.side_output(&tag, "bad-record".to_string()).unwrap();
        }

        assert_eq!(emitter.emissions.len(), 1);
        assert_eq!(emitter.emissions[0].0, "rejected");
        assert_eq!(emitter.emissions[0].1, EventTimestamp::new(100));
    }

    #[test]
    fn test_side_output_without_emitter_returns_error() {
        let mut timer_service = MockTimerService::new(EventTimestamp::new(0));
        let mut ctx = ProcessContext::new(EventTimestamp::new(100), None, &mut timer_service);

        let tag = OutputTag::<String>::new("rejected");
        let result = ctx.side_output(&tag, "bad-record".to_string());

        assert!(result.is_err());
    }
}
