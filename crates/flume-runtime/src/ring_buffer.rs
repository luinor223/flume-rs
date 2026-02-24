//! Lock-free SPSC ring buffer with cache-line padding.
//!
//! LMAX Disruptor-style single-producer single-consumer queue optimized
//! for the hot path between operators. Uses `tokio::sync::Notify` for
//! consumer parking when the buffer is empty.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_utils::CachePadded;
use tokio::sync::Notify;

/// Shared inner state of the ring buffer.
struct RingBufferInner<T> {
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: u64,
    write_seq: CachePadded<AtomicU64>,
    read_seq: CachePadded<AtomicU64>,
    closed: CachePadded<AtomicBool>,
    notify: Notify,
}

// Safety: only one thread writes and one thread reads; synchronization
// is handled via atomic sequences.
unsafe impl<T: Send> Send for RingBufferInner<T> {}
unsafe impl<T: Send> Sync for RingBufferInner<T> {}

impl<T> Drop for RingBufferInner<T> {
    fn drop(&mut self) {
        // Drop any initialized-but-unconsumed elements.
        let read = self.read_seq.load(Ordering::Relaxed);
        let write = self.write_seq.load(Ordering::Relaxed);
        for seq in read..write {
            let idx = (seq & self.mask) as usize;
            // Safety: these slots were written by the producer and not yet consumed.
            unsafe {
                (*self.buffer[idx].get()).assume_init_drop();
            }
        }
    }
}

/// Producer end of the ring buffer.
pub struct RingBufferProducer<T> {
    inner: Arc<RingBufferInner<T>>,
    /// Cached copy of the consumer's read sequence to avoid frequent
    /// atomic loads across cache lines.
    cached_read_seq: u64,
}

/// Consumer end of the ring buffer.
pub struct RingBufferConsumer<T> {
    inner: Arc<RingBufferInner<T>>,
    /// Cached copy of the producer's write sequence.
    cached_write_seq: u64,
}

/// Error returned when the ring buffer is full.
#[derive(Debug)]
pub struct Full<T>(pub T);

/// Create a new SPSC ring buffer with the given capacity (rounded up to
/// the next power of two).
pub fn ring_buffer<T>(capacity: usize) -> (RingBufferProducer<T>, RingBufferConsumer<T>) {
    let capacity = capacity.next_power_of_two().max(2);

    let mut buffer = Vec::with_capacity(capacity);
    for _ in 0..capacity {
        buffer.push(UnsafeCell::new(MaybeUninit::uninit()));
    }

    let inner = Arc::new(RingBufferInner {
        buffer: buffer.into_boxed_slice(),
        mask: (capacity - 1) as u64,
        write_seq: CachePadded::new(AtomicU64::new(0)),
        read_seq: CachePadded::new(AtomicU64::new(0)),
        closed: CachePadded::new(AtomicBool::new(false)),
        notify: Notify::new(),
    });

    let producer = RingBufferProducer {
        inner: Arc::clone(&inner),
        cached_read_seq: 0,
    };
    let consumer = RingBufferConsumer {
        inner,
        cached_write_seq: 0,
    };

    (producer, consumer)
}

impl<T> RingBufferProducer<T> {
    /// Try to enqueue a value. Returns `Err(Full(value))` if the buffer is full.
    pub fn try_send(&mut self, value: T) -> Result<(), Full<T>> {
        let write = self.inner.write_seq.load(Ordering::Relaxed);

        // Check if there's room. First try the cached read_seq.
        if write - self.cached_read_seq > self.inner.mask {
            // Cache is stale — reload from the atomic.
            self.cached_read_seq = self.inner.read_seq.load(Ordering::Acquire);
            if write - self.cached_read_seq > self.inner.mask {
                return Err(Full(value));
            }
        }

        let idx = (write & self.inner.mask) as usize;
        // Safety: we're the only writer and we've confirmed the slot is free.
        unsafe {
            (*self.inner.buffer[idx].get()).write(value);
        }

        // Publish the write with Release ordering so the consumer sees the data.
        self.inner.write_seq.store(write + 1, Ordering::Release);

        // Notify the consumer if this is the first element (empty→non-empty).
        if write == self.cached_read_seq {
            self.inner.notify.notify_one();
        }

        Ok(())
    }
}

impl<T> Drop for RingBufferProducer<T> {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.notify.notify_one();
    }
}

impl<T> RingBufferConsumer<T> {
    /// Try to dequeue a value without blocking. Returns `None` if empty.
    pub fn try_recv(&mut self) -> Option<T> {
        let read = self.inner.read_seq.load(Ordering::Relaxed);

        if read >= self.cached_write_seq {
            self.cached_write_seq = self.inner.write_seq.load(Ordering::Acquire);
            if read >= self.cached_write_seq {
                return None;
            }
        }

        let idx = (read & self.inner.mask) as usize;
        // Safety: the producer has published this slot.
        let value = unsafe { (*self.inner.buffer[idx].get()).assume_init_read() };

        // Advance the read sequence.
        self.inner.read_seq.store(read + 1, Ordering::Release);

        Some(value)
    }

    /// Async receive: spins briefly, then parks on `Notify`.
    /// Returns `None` when the producer is dropped and the buffer is drained.
    pub async fn recv(&mut self) -> Option<T> {
        const SPIN_LIMIT: u32 = 32;

        // Fast path: try_recv.
        if let Some(value) = self.try_recv() {
            return Some(value);
        }

        loop {
            // Spin a few times before parking.
            for _ in 0..SPIN_LIMIT {
                if let Some(value) = self.try_recv() {
                    return Some(value);
                }
                std::hint::spin_loop();
            }

            // Check closed before parking to avoid missed wakeup.
            if self.inner.closed.load(Ordering::Acquire) {
                // Drain any remaining elements.
                return self.try_recv();
            }

            // Park until notified.
            self.inner.notify.notified().await;

            // After wakeup, try again.
            if let Some(value) = self.try_recv() {
                return Some(value);
            }

            // Could be a spurious wakeup or a close signal.
            if self.inner.closed.load(Ordering::Acquire) {
                return self.try_recv();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_send_recv_sync() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(4);

        producer.try_send(1).unwrap();
        producer.try_send(2).unwrap();
        producer.try_send(3).unwrap();

        assert_eq!(consumer.try_recv(), Some(1));
        assert_eq!(consumer.try_recv(), Some(2));
        assert_eq!(consumer.try_recv(), Some(3));
        assert_eq!(consumer.try_recv(), None);
    }

    #[test]
    fn test_capacity_rounds_to_power_of_two() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(3);

        // Capacity is 4 (next power of two from 3).
        producer.try_send(1).unwrap();
        producer.try_send(2).unwrap();
        producer.try_send(3).unwrap();
        producer.try_send(4).unwrap();

        // Buffer is now full.
        assert!(producer.try_send(5).is_err());

        assert_eq!(consumer.try_recv(), Some(1));
        assert_eq!(consumer.try_recv(), Some(2));
        assert_eq!(consumer.try_recv(), Some(3));
        assert_eq!(consumer.try_recv(), Some(4));
    }

    #[test]
    fn test_full_buffer_returns_error() {
        let (mut producer, _consumer) = ring_buffer::<i32>(2);

        producer.try_send(1).unwrap();
        producer.try_send(2).unwrap();

        let err = producer.try_send(3);
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().0, 3);
    }

    #[tokio::test]
    async fn test_async_recv() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(4);

        producer.try_send(42).unwrap();

        let val = consumer.recv().await;
        assert_eq!(val, Some(42));
    }

    #[tokio::test]
    async fn test_producer_drop_signals_consumer() {
        let (producer, mut consumer) = ring_buffer::<i32>(4);

        drop(producer);

        let val = consumer.recv().await;
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_producer_drop_after_send() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(4);

        producer.try_send(1).unwrap();
        producer.try_send(2).unwrap();
        drop(producer);

        assert_eq!(consumer.recv().await, Some(1));
        assert_eq!(consumer.recv().await, Some(2));
        assert_eq!(consumer.recv().await, None);
    }

    #[tokio::test]
    async fn test_concurrent_spsc() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(16);
        let count = 10_000;

        let producer_handle = tokio::spawn(async move {
            for i in 0..count {
                loop {
                    match producer.try_send(i) {
                        Ok(()) => break,
                        Err(_) => tokio::task::yield_now().await,
                    }
                }
            }
            // Drop producer to signal completion.
        });

        let consumer_handle = tokio::spawn(async move {
            let mut received = Vec::with_capacity(count as usize);
            while let Some(val) = consumer.recv().await {
                received.push(val);
            }
            received
        });

        producer_handle.await.unwrap();
        let received = consumer_handle.await.unwrap();

        assert_eq!(received.len(), count as usize);
        for (i, val) in received.iter().enumerate() {
            assert_eq!(*val, i as i32);
        }
    }

    #[test]
    fn test_wraparound() {
        let (mut producer, mut consumer) = ring_buffer::<i32>(2);

        // Fill and drain multiple times to test wraparound.
        for round in 0..10 {
            producer.try_send(round * 2).unwrap();
            producer.try_send(round * 2 + 1).unwrap();

            assert_eq!(consumer.try_recv(), Some(round * 2));
            assert_eq!(consumer.try_recv(), Some(round * 2 + 1));
        }
    }

    #[test]
    fn test_drop_cleans_up_unconsumed() {
        // This test verifies no memory leaks via Drop.
        let (mut producer, consumer) = ring_buffer::<String>(4);

        producer.try_send("a".to_string()).unwrap();
        producer.try_send("b".to_string()).unwrap();
        producer.try_send("c".to_string()).unwrap();

        // Only consume one.
        let mut consumer = consumer;
        assert_eq!(consumer.try_recv(), Some("a".to_string()));

        // Drop with 2 unconsumed — should not leak.
        drop(consumer);
        drop(producer);
    }
}
