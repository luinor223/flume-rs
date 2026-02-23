//! WindowedStream — a keyed stream with a window assigner, ready for aggregation.

use flume_core::{AggregateFunction, FlumeResult, Sink, Trigger, WindowAssigner, WindowOperator};

use crate::datastream::DataStream;

/// A keyed, windowed stream. Created by [`DataStream::window`]. Call
/// [`aggregate`](WindowedStream::aggregate) to apply an aggregate function.
pub struct WindowedStream<'env, T: Send + 'static> {
    stream: DataStream<'env, T>,
    assigner: Box<dyn WindowAssigner>,
    trigger: Option<Box<dyn Trigger>>,
}

impl<'env, T: Send + 'static> WindowedStream<'env, T> {
    pub(crate) fn new(stream: DataStream<'env, T>, assigner: Box<dyn WindowAssigner>) -> Self {
        Self {
            stream,
            assigner,
            trigger: None,
        }
    }

    /// Override the default trigger (EventTimeTrigger).
    pub fn trigger(mut self, trigger: Box<dyn Trigger>) -> Self {
        self.trigger = trigger.into();
        self
    }

    /// Apply an aggregate function to this windowed stream, producing a new
    /// DataStream of the aggregate output type.
    pub fn aggregate<Acc, Out, Agg>(self, agg: Agg) -> DataStream<'env, Out>
    where
        Acc: Send + 'static,
        Out: Send + 'static,
        Agg: AggregateFunction<T, Acc, Out> + 'static,
    {
        let mut op = WindowOperator::new(self.assigner, agg);
        if let Some(trigger) = self.trigger {
            op = op.with_trigger(trigger);
        }
        self.stream.apply_operator("window", op)
    }

    /// Terminal: aggregate and immediately sink the results.
    pub async fn add_sink<Acc, Out, Agg, S>(self, agg: Agg, sink: S) -> FlumeResult<()>
    where
        Acc: Send + 'static,
        Out: Send + 'static,
        Agg: AggregateFunction<T, Acc, Out> + 'static,
        S: Sink<Out> + 'static,
    {
        self.aggregate(agg).add_sink(sink).await
    }
}
