//! Timer queue and timer service implementation for process functions.
//!
//! Uses `BTreeSet` for ordered storage of timers, which provides automatic
//! deduplication and efficient range queries for firing timers up to a watermark.

use std::collections::BTreeSet;

use crate::{EventTimestamp, TimerService};

/// An ordered, deduplicated queue of timers keyed by `(timestamp, key_bytes)`.
///
/// Maintains separate sets for event-time and processing-time timers.
pub struct TimerQueue {
    event_time_timers: BTreeSet<(EventTimestamp, Vec<u8>)>,
    processing_time_timers: BTreeSet<(EventTimestamp, Vec<u8>)>,
}

impl TimerQueue {
    /// Create an empty timer queue.
    pub fn new() -> Self {
        Self {
            event_time_timers: BTreeSet::new(),
            processing_time_timers: BTreeSet::new(),
        }
    }

    /// Register an event-time timer for the given timestamp and key.
    /// Duplicate registrations are automatically ignored (BTreeSet dedup).
    pub fn register_event_time(&mut self, timestamp: EventTimestamp, key: Vec<u8>) {
        self.event_time_timers.insert((timestamp, key));
    }

    /// Delete an event-time timer for the given timestamp and key.
    pub fn delete_event_time(&mut self, timestamp: EventTimestamp, key: Vec<u8>) {
        self.event_time_timers.remove(&(timestamp, key));
    }

    /// Fire all event-time timers with timestamp <= `watermark`.
    /// Returns the fired timers in ascending order and removes them from the queue.
    pub fn fire_event_time_up_to(
        &mut self,
        watermark: EventTimestamp,
    ) -> Vec<(EventTimestamp, Vec<u8>)> {
        let split_point = (
            EventTimestamp::new(watermark.as_millis().saturating_add(1)),
            Vec::new(),
        );
        let remaining = self.event_time_timers.split_off(&split_point);
        let fired: Vec<_> = self.event_time_timers.iter().cloned().collect();
        self.event_time_timers = remaining;
        fired
    }

    /// Register a processing-time timer for the given timestamp and key.
    pub fn register_processing_time(&mut self, timestamp: EventTimestamp, key: Vec<u8>) {
        self.processing_time_timers.insert((timestamp, key));
    }

    /// Delete a processing-time timer for the given timestamp and key.
    pub fn delete_processing_time(&mut self, timestamp: EventTimestamp, key: Vec<u8>) {
        self.processing_time_timers.remove(&(timestamp, key));
    }

    /// Fire all processing-time timers with timestamp <= `time`.
    /// Returns the fired timers in ascending order and removes them from the queue.
    pub fn fire_processing_time_up_to(
        &mut self,
        time: EventTimestamp,
    ) -> Vec<(EventTimestamp, Vec<u8>)> {
        let split_point = (
            EventTimestamp::new(time.as_millis().saturating_add(1)),
            Vec::new(),
        );
        let remaining = self.processing_time_timers.split_off(&split_point);
        let fired: Vec<_> = self.processing_time_timers.iter().cloned().collect();
        self.processing_time_timers = remaining;
        fired
    }
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Concrete implementation of [`TimerService`] backed by a [`TimerQueue`].
///
/// Tracks the current watermark and the current key (set externally before
/// each `process_element` or `on_timer` call). Timer registrations are
/// automatically associated with the current key.
pub struct TimerServiceImpl {
    queue: TimerQueue,
    current_watermark: EventTimestamp,
    current_key: Vec<u8>,
}

impl TimerServiceImpl {
    /// Create a new timer service with the given initial watermark.
    pub fn new() -> Self {
        Self {
            queue: TimerQueue::new(),
            current_watermark: EventTimestamp::MIN,
            current_key: Vec::new(),
        }
    }

    /// Set the current key for subsequent timer registrations.
    pub fn set_current_key(&mut self, key: Vec<u8>) {
        self.current_key = key;
    }

    /// Update the current watermark.
    pub fn set_watermark(&mut self, watermark: EventTimestamp) {
        self.current_watermark = watermark;
    }

    /// Access the underlying timer queue (e.g. to fire timers).
    pub fn queue_mut(&mut self) -> &mut TimerQueue {
        &mut self.queue
    }
}

impl Default for TimerServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerService for TimerServiceImpl {
    fn current_watermark(&self) -> EventTimestamp {
        self.current_watermark
    }

    fn register_event_time_timer(&mut self, time: EventTimestamp) {
        self.queue
            .register_event_time(time, self.current_key.clone());
    }

    fn delete_event_time_timer(&mut self, time: EventTimestamp) {
        self.queue.delete_event_time(time, self.current_key.clone());
    }

    fn register_processing_time_timer(&mut self, time: EventTimestamp) {
        self.queue
            .register_processing_time(time, self.current_key.clone());
    }

    fn delete_processing_time_timer(&mut self, time: EventTimestamp) {
        self.queue
            .delete_processing_time(time, self.current_key.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_fire_event_time_timer() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(100), vec![1]);
        queue.register_event_time(EventTimestamp::new(200), vec![1]);

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(150));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, EventTimestamp::new(100));

        // Remaining timer should still be in the queue.
        let fired = queue.fire_event_time_up_to(EventTimestamp::new(200));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, EventTimestamp::new(200));
    }

    #[test]
    fn test_fire_returns_ascending_order() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(300), vec![1]);
        queue.register_event_time(EventTimestamp::new(100), vec![1]);
        queue.register_event_time(EventTimestamp::new(200), vec![1]);

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(300));
        let timestamps: Vec<u64> = fired.iter().map(|(t, _)| t.as_millis()).collect();
        assert_eq!(timestamps, vec![100, 200, 300]);
    }

    #[test]
    fn test_deduplication() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(100), vec![1]);
        queue.register_event_time(EventTimestamp::new(100), vec![1]); // duplicate

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(100));
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn test_different_keys_not_deduplicated() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(100), vec![1]);
        queue.register_event_time(EventTimestamp::new(100), vec![2]);

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(100));
        assert_eq!(fired.len(), 2);
    }

    #[test]
    fn test_delete_event_time_timer() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(100), vec![1]);
        queue.register_event_time(EventTimestamp::new(200), vec![1]);

        queue.delete_event_time(EventTimestamp::new(100), vec![1]);

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(200));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, EventTimestamp::new(200));
    }

    #[test]
    fn test_fire_empty_queue() {
        let mut queue = TimerQueue::new();
        let fired = queue.fire_event_time_up_to(EventTimestamp::new(1000));
        assert!(fired.is_empty());
    }

    #[test]
    fn test_fire_none_eligible() {
        let mut queue = TimerQueue::new();
        queue.register_event_time(EventTimestamp::new(500), vec![1]);

        let fired = queue.fire_event_time_up_to(EventTimestamp::new(100));
        assert!(fired.is_empty());

        // Timer should still be there.
        let fired = queue.fire_event_time_up_to(EventTimestamp::new(500));
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn test_processing_time_timers() {
        let mut queue = TimerQueue::new();
        queue.register_processing_time(EventTimestamp::new(100), vec![1]);
        queue.register_processing_time(EventTimestamp::new(200), vec![1]);

        let fired = queue.fire_processing_time_up_to(EventTimestamp::new(150));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0, EventTimestamp::new(100));
    }

    #[test]
    fn test_timer_service_impl() {
        let mut service = TimerServiceImpl::new();
        assert_eq!(service.current_watermark(), EventTimestamp::MIN);

        service.set_current_key(vec![1]);
        service.register_event_time_timer(EventTimestamp::new(100));
        service.register_event_time_timer(EventTimestamp::new(200));

        service.set_watermark(EventTimestamp::new(150));
        assert_eq!(service.current_watermark(), EventTimestamp::new(150));

        let fired = service
            .queue_mut()
            .fire_event_time_up_to(EventTimestamp::new(150));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0], (EventTimestamp::new(100), vec![1]));
    }

    #[test]
    fn test_timer_service_delete() {
        let mut service = TimerServiceImpl::new();
        service.set_current_key(vec![1]);
        service.register_event_time_timer(EventTimestamp::new(100));
        service.delete_event_time_timer(EventTimestamp::new(100));

        let fired = service
            .queue_mut()
            .fire_event_time_up_to(EventTimestamp::new(200));
        assert!(fired.is_empty());
    }

    #[test]
    fn test_timer_service_per_key_timers() {
        let mut service = TimerServiceImpl::new();

        service.set_current_key(vec![1]);
        service.register_event_time_timer(EventTimestamp::new(100));

        service.set_current_key(vec![2]);
        service.register_event_time_timer(EventTimestamp::new(100));

        let fired = service
            .queue_mut()
            .fire_event_time_up_to(EventTimestamp::new(100));
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[0].1, vec![1]);
        assert_eq!(fired[1].1, vec![2]);
    }
}
