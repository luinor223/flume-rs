//! JoinStream and WindowedJoinStream — window join API.
//!
//! Created by [`DataStream::window_join`]. Buffers two inputs and applies
//! a join or cogroup function when the window fires.

use std::hash::Hash;

use flume_core::{
    CogroupFunction, JoinFunction, Source, WindowAssigner, WindowCogroupOperator,
    WindowJoinOperator,
};

use crate::connected::{KeyExtractor, hash_key, wire_two_input_operator};
use crate::datastream::DataStream;

/// A two-input stream ready for window-based joining.
///
/// Created by [`DataStream::window_join`]. Call [`key_by_both`](Self::key_by_both)
/// to set key extractors, then [`window`](Self::window) to assign a window.
pub struct JoinStream<'env, In1: Send + 'static, In2: Send + 'static> {
    stream: DataStream<'env, In1>,
    source: Box<dyn Source<In2>>,
    key_extractor_1: Option<KeyExtractor<In1>>,
    key_extractor_2: Option<KeyExtractor<In2>>,
}

impl<'env, In1: Send + 'static, In2: Send + 'static> JoinStream<'env, In1, In2> {
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

    /// Assign a window to this join stream.
    pub fn window(
        self,
        assigner: impl WindowAssigner + 'static,
    ) -> WindowedJoinStream<'env, In1, In2> {
        WindowedJoinStream {
            stream: self.stream,
            source: self.source,
            key_extractor_1: self.key_extractor_1,
            key_extractor_2: self.key_extractor_2,
            assigner: Box::new(assigner),
        }
    }
}

/// A windowed join stream ready for applying a join or cogroup function.
pub struct WindowedJoinStream<'env, In1: Send + 'static, In2: Send + 'static> {
    stream: DataStream<'env, In1>,
    source: Box<dyn Source<In2>>,
    key_extractor_1: Option<KeyExtractor<In1>>,
    key_extractor_2: Option<KeyExtractor<In2>>,
    assigner: Box<dyn WindowAssigner>,
}

impl<'env, In1: Send + Clone + 'static, In2: Send + Clone + 'static>
    WindowedJoinStream<'env, In1, In2>
{
    /// Apply a join function to produce the cross product of matching
    /// left and right records within each window.
    pub fn apply<Out, JF>(self, join_fn: JF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        JF: JoinFunction<In1, In2, Out> + 'static,
    {
        let operator = WindowJoinOperator::new(self.assigner, join_fn);
        wire_two_input_operator(
            self.stream,
            self.source,
            self.key_extractor_1,
            self.key_extractor_2,
            "window_join",
            operator,
        )
    }

    /// Apply a cogroup function that receives all left and right records
    /// for each (key, window) as slices.
    pub fn cogroup<Out, CF>(self, cogroup_fn: CF) -> DataStream<'env, Out>
    where
        Out: Send + 'static,
        CF: CogroupFunction<In1, In2, Out> + 'static,
    {
        let operator = WindowCogroupOperator::new(self.assigner, cogroup_fn);
        wire_two_input_operator(
            self.stream,
            self.source,
            self.key_extractor_1,
            self.key_extractor_2,
            "window_cogroup",
            operator,
        )
    }
}
