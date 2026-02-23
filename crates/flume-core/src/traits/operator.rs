//! Operator and Collector traits for the hot path.

use crate::{CheckpointBarrier, FlumeResult, Record, Watermark};

/// Decouples operators from the transport layer. Operators push output
/// through a Collector without knowing whether it's backed by a channel,
/// a ring buffer, or a test harness.
pub trait Collector<T: Send>: Send {
    fn collect(&mut self, record: Record<T>) -> FlumeResult<()>;
    fn collect_watermark(&mut self, watermark: Watermark) -> FlumeResult<()>;
    fn collect_barrier(&mut self, barrier: CheckpointBarrier) -> FlumeResult<()>;
}

/// The hot-path trait. Processes records synchronously, one at a time.
///
/// Synchronous by design — an async `process_record` would compile each
/// operator into a state machine, adding overhead for operators that just
/// call `f(value)`. The wrapping TaskExecutor is async; operators are not.
pub trait Operator<In: Send, Out: Send>: Send {
    /// Process a single data record.
    fn process_record(
        &mut self,
        record: Record<In>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()>;

    /// Process a watermark. Default: forward unchanged.
    fn process_watermark(
        &mut self,
        watermark: Watermark,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        collector.collect_watermark(watermark)
    }

    /// Process a checkpoint barrier. Default: forward unchanged.
    fn process_barrier(
        &mut self,
        barrier: CheckpointBarrier,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        collector.collect_barrier(barrier)
    }
}
