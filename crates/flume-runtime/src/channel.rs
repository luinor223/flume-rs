//! Channel-backed transport between operators.

use flume_core::{
    CheckpointBarrier, Collector, FlumeError, FlumeResult, Record, StreamElement, Watermark,
};
use tokio::sync::mpsc;

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

/// Create a paired collector + receiver for connecting operators.
pub fn operator_channel<T: Send + 'static>(
    buffer_size: usize,
) -> (ChannelCollector<T>, mpsc::Receiver<StreamElement<T>>) {
    let (tx, rx) = mpsc::channel(buffer_size);
    (ChannelCollector::new(tx), rx)
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
}
