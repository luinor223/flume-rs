//! Join function traits for window joins and interval joins.

use crate::{Collector, FlumeResult, Record};

/// A function that joins matching elements from two input streams.
///
/// Used by both window join (cross-product within a window) and
/// interval join (range-based matching).
pub trait JoinFunction<In1, In2, Out>: Send {
    fn join(&mut self, left: &In1, right: &In2) -> Out;
}

impl<In1, In2, Out, F> JoinFunction<In1, In2, Out> for F
where
    F: FnMut(&In1, &In2) -> Out + Send,
{
    fn join(&mut self, left: &In1, right: &In2) -> Out {
        (self)(left, right)
    }
}

/// A function that receives all elements from both sides for a given key
/// and window, and produces zero or more outputs.
pub trait CogroupFunction<In1, In2, Out>: Send {
    fn cogroup(&mut self, key: &[u8], left: &[In1], right: &[In2]) -> Vec<Out>;
}

impl<In1, In2, Out, F> CogroupFunction<In1, In2, Out> for F
where
    F: FnMut(&[u8], &[In1], &[In2]) -> Vec<Out> + Send,
{
    fn cogroup(&mut self, key: &[u8], left: &[In1], right: &[In2]) -> Vec<Out> {
        (self)(key, left, right)
    }
}

/// Emit each element from `results` as a record with the given key and timestamp.
pub(crate) fn emit_results<Out: Send>(
    results: Vec<Out>,
    key: Vec<u8>,
    timestamp: crate::EventTimestamp,
    collector: &mut dyn Collector<Out>,
) -> FlumeResult<()> {
    for value in results {
        let record = Record::new(value, timestamp).with_key(key.clone());
        collector.collect(record)?;
    }
    Ok(())
}
