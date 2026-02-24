//! Channel-backed transport between operators.

use flume_core::{
    CheckpointBarrier, Collector, FlumeError, FlumeResult, Record, StreamElement, Watermark,
};
use tokio::sync::mpsc;

use crate::ring_buffer::{self, RingBufferConsumer, RingBufferProducer};

/// Implements [`Collector`] backed by a bounded `tokio::sync::mpsc::Sender`.
///
/// Uses `try_send` (non-blocking) because the calling operator is synchronous.
/// If the channel is full, returns [`FlumeError::ChannelClosed`].
pub struct ChannelCollector<T: Send + 'static> {
    tx: mpsc::Sender<StreamElement<T>>,
}

impl<T: Send + 'static> ChannelCollector<T> {
    pub fn new(tx: mpsc::Sender<StreamElement<T>>) -> Self {
        Self { tx }
    }

    fn try_send(&self, element: StreamElement<T>) -> FlumeResult<()> {
        self.tx
            .try_send(element)
            .map_err(|_| FlumeError::ChannelClosed)
    }
}

impl<T: Send + 'static> Collector<T> for ChannelCollector<T> {
    fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
        self.try_send(StreamElement::Record(record))
    }

    fn collect_watermark(&mut self, watermark: Watermark) -> FlumeResult<()> {
        self.try_send(StreamElement::Watermark(watermark))
    }

    fn collect_barrier(&mut self, barrier: CheckpointBarrier) -> FlumeResult<()> {
        self.try_send(StreamElement::CheckpointBarrier(barrier))
    }
}

/// Implements [`Collector`] backed by a [`RingBufferProducer`].
pub struct RingBufferCollector<T: Send + 'static> {
    producer: RingBufferProducer<StreamElement<T>>,
}

impl<T: Send + 'static> RingBufferCollector<T> {
    pub fn new(producer: RingBufferProducer<StreamElement<T>>) -> Self {
        Self { producer }
    }

    fn try_send(&mut self, element: StreamElement<T>) -> FlumeResult<()> {
        self.producer
            .try_send(element)
            .map_err(|_| FlumeError::ChannelClosed)
    }
}

impl<T: Send + 'static> Collector<T> for RingBufferCollector<T> {
    fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
        self.try_send(StreamElement::Record(record))
    }

    fn collect_watermark(&mut self, watermark: Watermark) -> FlumeResult<()> {
        self.try_send(StreamElement::Watermark(watermark))
    }

    fn collect_barrier(&mut self, barrier: CheckpointBarrier) -> FlumeResult<()> {
        self.try_send(StreamElement::CheckpointBarrier(barrier))
    }
}

/// Abstraction over the receive side of an operator channel.
///
/// Wraps either a `tokio::sync::mpsc::Receiver` or a [`RingBufferConsumer`],
/// providing a uniform async `recv()` interface for [`TaskExecutor`](crate::task::TaskExecutor).
pub enum OperatorInput<T: Send + 'static> {
    Mpsc(mpsc::Receiver<StreamElement<T>>),
    RingBuffer(RingBufferConsumer<StreamElement<T>>),
}

impl<T: Send + 'static> OperatorInput<T> {
    /// Receive the next stream element, or `None` if the channel is closed.
    pub async fn recv(&mut self) -> Option<StreamElement<T>> {
        match self {
            OperatorInput::Mpsc(rx) => rx.recv().await,
            OperatorInput::RingBuffer(consumer) => consumer.recv().await,
        }
    }
}

/// Create a paired collector + receiver for connecting operators.
pub fn operator_channel<T: Send + 'static>(
    buffer_size: usize,
) -> (ChannelCollector<T>, mpsc::Receiver<StreamElement<T>>) {
    let (tx, rx) = mpsc::channel(buffer_size);
    (ChannelCollector::new(tx), rx)
}

/// Create a paired collector + `OperatorInput` backed by the SPSC ring buffer.
pub fn operator_ring_buffer<T: Send + 'static>(
    buffer_size: usize,
) -> (RingBufferCollector<T>, OperatorInput<T>) {
    let (producer, consumer) = ring_buffer::ring_buffer(buffer_size);
    (
        RingBufferCollector::new(producer),
        OperatorInput::RingBuffer(consumer),
    )
}

/// Create a paired collector + `OperatorInput` using the specified channel kind.
pub fn operator_channel_with_kind<T: Send + 'static>(
    buffer_size: usize,
    kind: flume_core::ChannelKind,
) -> (Box<dyn Collector<T>>, OperatorInput<T>) {
    match kind {
        flume_core::ChannelKind::Mpsc => {
            let (collector, rx) = operator_channel(buffer_size);
            (Box::new(collector), OperatorInput::Mpsc(rx))
        }
        flume_core::ChannelKind::RingBuffer => {
            let (collector, input) = operator_ring_buffer(buffer_size);
            (Box::new(collector), input)
        }
    }
}

#[cfg(test)]
mod tests {
    use flume_core::EventTimestamp;

    use super::*;

    #[tokio::test]
    async fn test_channel_collector_send_receive() {
        let (mut collector, mut rx) = operator_channel::<i32>(16);

        collector
            .collect(Record::new(42, EventTimestamp::new(100)))
            .unwrap();
        collector
            .collect_watermark(Watermark::new(EventTimestamp::new(200)))
            .unwrap();

        let elem = rx.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 42));

        let elem = rx.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Watermark(_)));
    }

    #[tokio::test]
    async fn test_channel_full_returns_error() {
        let (mut collector, _rx) = operator_channel::<i32>(1);

        // Fill the channel
        collector
            .collect(Record::new(1, EventTimestamp::new(100)))
            .unwrap();

        // Second send should fail (buffer full)
        let result = collector.collect(Record::new(2, EventTimestamp::new(200)));
        assert!(matches!(result, Err(FlumeError::ChannelClosed)));
    }

    #[tokio::test]
    async fn test_channel_closed_returns_error() {
        let (mut collector, rx) = operator_channel::<i32>(16);
        drop(rx);

        let result = collector.collect(Record::new(1, EventTimestamp::new(100)));
        assert!(matches!(result, Err(FlumeError::ChannelClosed)));
    }

    #[tokio::test]
    async fn test_ring_buffer_collector_send_receive() {
        let (mut collector, mut input) = operator_ring_buffer::<i32>(16);

        collector
            .collect(Record::new(42, EventTimestamp::new(100)))
            .unwrap();
        collector
            .collect_watermark(Watermark::new(EventTimestamp::new(200)))
            .unwrap();

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 42));

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Watermark(_)));
    }

    #[tokio::test]
    async fn test_operator_input_mpsc() {
        let (collector, rx) = operator_channel::<i32>(16);
        let mut input = OperatorInput::Mpsc(rx);

        let mut collector: Box<dyn Collector<i32>> = Box::new(collector);
        collector
            .collect(Record::new(99, EventTimestamp::new(100)))
            .unwrap();
        drop(collector);

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 99));

        let none = input.recv().await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn test_operator_input_ring_buffer() {
        let (mut collector, mut input) = operator_ring_buffer::<i32>(16);

        collector
            .collect(Record::new(77, EventTimestamp::new(100)))
            .unwrap();
        drop(collector);

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 77));

        let none = input.recv().await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn test_operator_channel_with_kind_mpsc() {
        let (mut collector, mut input) =
            operator_channel_with_kind::<i32>(16, flume_core::ChannelKind::Mpsc);

        collector
            .collect(Record::new(10, EventTimestamp::new(100)))
            .unwrap();
        drop(collector);

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 10));
    }

    #[tokio::test]
    async fn test_operator_channel_with_kind_ring_buffer() {
        let (mut collector, mut input) =
            operator_channel_with_kind::<i32>(16, flume_core::ChannelKind::RingBuffer);

        collector
            .collect(Record::new(20, EventTimestamp::new(100)))
            .unwrap();
        drop(collector);

        let elem = input.recv().await.unwrap();
        assert!(matches!(elem, StreamElement::Record(r) if r.value == 20));
    }
}
