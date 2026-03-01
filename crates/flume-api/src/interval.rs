//! IntervalJoinStream — interval join API.
//!
//! Created by [`DataStream::interval_join`]. Matches records from two streams
//! based on event-time proximity within configurable bounds.

use std::hash::Hash;
use std::time::Duration;

use flume_core::{IntervalJoinOperator, JoinFunction, Source};

use crate::connected::{KeyExtractor, hash_key, wire_two_input_operator};
use crate::datastream::DataStream;

/// A two-input stream ready for interval-based joining.
///
/// Created by [`DataStream::interval_join`]. Call [`key_by_both`](Self::key_by_both)
/// to set key extractors, then [`between`](Self::between) to set the time bounds.
pub struct IntervalJoinStream<'env, In1: Send + 'static, In2: Send + 'static> {
    stream: DataStream<'env, In1>,
    source: Box<dyn Source<In2>>,
    key_extractor_1: Option<KeyExtractor<In1>>,
    key_extractor_2: Option<KeyExtractor<In2>>,
}

impl<'env, In1: Send + 'static, In2: Send + 'static> IntervalJoinStream<'env, In1, In2> {
    pub(crate) fn new(stream: DataStream<'env, In1>, source: Box<dyn Source<In2>>) -> Self {
        Self {
            stream,
            source,
            key_extractor_1: None,
            key_extractor_2: None,
        }
    }

    /// Set key extractors for both inputs.
    pub fn key_by_both<K1, K2, F1, F2>(mut self, mut f1: F1, mut f2: F2) -> Self
    where
        K1: Hash + Send + 'static,
        K2: Hash + Send + 'static,
        F1: FnMut(&In1) -> K1 + Send + 'static,
        F2: FnMut(&In2) -> K2 + Send + 'static,
    {
        self.key_extractor_1 = Some(Box::new(move |value: &In1| hash_key(&f1(value))));
        self.key_extractor_2 = Some(Box::new(move |value: &In2| hash_key(&f2(value))));
        self
    }

    /// Set the time bounds for the interval join.
    ///
    /// A left record at `t_l` matches right records at `t_r` where
    /// `t_l + lower_bound <= t_r <= t_l + upper_bound`.
    pub fn between(
        self,
        lower_bound: Duration,
        upper_bound: Duration,
    ) -> BoundedIntervalJoinStream<'env, In1, In2> {
        BoundedIntervalJoinStream {
            stream: self.stream,
            source: self.source,
            key_extractor_1: self.key_extractor_1,
            key_extractor_2: self.key_extractor_2,
            lower_bound,
            upper_bound,
        }
    }
}

/// An interval join stream with bounds set, ready for applying a join function.
pub struct BoundedIntervalJoinStream<'env, In1: Send + 'static, In2: Send + 'static> {
    stream: DataStream<'env, In1>,
    source: Box<dyn Source<In2>>,
    key_extractor_1: Option<KeyExtractor<In1>>,
    key_extractor_2: Option<KeyExtractor<In2>>,
    lower_bound: Duration,
    upper_bound: Duration,
}

impl<'env, In1: Send + Clone + 'static, In2: Send + Clone + 'static>
    BoundedIntervalJoinStream<'env, In1, In2>
{
    /// Apply a join function to matching record pairs within the interval bounds.
    pub fn process<Out, JF>(self, join_fn: JF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        JF: JoinFunction<In1, In2, Out> + 'static,
    {
        let operator = IntervalJoinOperator::new(self.lower_bound, self.upper_bound, join_fn);
        wire_two_input_operator(
            self.stream,
            self.source,
            self.key_extractor_1,
            self.key_extractor_2,
            "interval_join",
            operator,
        )
    }
}
