//! Window types, assigners, triggers, and aggregate functions.

use std::time::Duration;

use crate::EventTimestamp;

/// A time range `[start, end)` — inclusive start, exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    pub start: EventTimestamp,
    pub end: EventTimestamp,
}

impl Window {
    pub fn new(start: EventTimestamp, end: EventTimestamp) -> Self {
        Self { start, end }
    }

    /// Check if a timestamp falls within this window.
    pub fn contains(&self, timestamp: EventTimestamp) -> bool {
        timestamp >= self.start && timestamp < self.end
    }
}

// ---------------------------------------------------------------------------
// Window Assigners
// ---------------------------------------------------------------------------

/// Assigns records to one or more windows based on their event timestamp.
pub trait WindowAssigner: Send {
    fn assign_windows(&self, timestamp: EventTimestamp) -> Vec<Window>;
}

/// Non-overlapping, fixed-size windows. Every record belongs to exactly one window.
#[derive(Debug, Clone)]
pub struct TumblingWindow {
    size_ms: u64,
}

impl TumblingWindow {
    pub fn new(size: Duration) -> Self {
        Self {
            size_ms: size.as_millis() as u64,
        }
    }
}

impl WindowAssigner for TumblingWindow {
    fn assign_windows(&self, timestamp: EventTimestamp) -> Vec<Window> {
        let ts = timestamp.as_millis();
        let start = (ts / self.size_ms) * self.size_ms;
        vec![Window::new(
            EventTimestamp::new(start),
            EventTimestamp::new(start + self.size_ms),
        )]
    }
}

/// Overlapping windows. A record may belong to multiple windows.
#[derive(Debug, Clone)]
pub struct SlidingWindow {
    size_ms: u64,
    slide_ms: u64,
}

impl SlidingWindow {
    pub fn new(size: Duration, slide: Duration) -> Self {
        Self {
            size_ms: size.as_millis() as u64,
            slide_ms: slide.as_millis() as u64,
        }
    }
}

impl WindowAssigner for SlidingWindow {
    fn assign_windows(&self, timestamp: EventTimestamp) -> Vec<Window> {
        let ts = timestamp.as_millis();
        let mut windows = Vec::new();

        // Find the latest window start that is <= ts
        let last_start = (ts / self.slide_ms) * self.slide_ms;

        // Walk backwards, collecting all windows that contain this timestamp
        let mut start = last_start;
        loop {
            let end = start + self.size_ms;
            if ts < end {
                windows.push(Window::new(
                    EventTimestamp::new(start),
                    EventTimestamp::new(end),
                ));
            }
            if start < self.slide_ms {
                break;
            }
            start -= self.slide_ms;
            // Stop if the window end is before the timestamp
            if start + self.size_ms <= ts {
                break;
            }
        }

        windows.reverse();
        windows
    }
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

/// Result of a trigger evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerResult {
    /// Do nothing.
    Continue,
    /// Compute result, keep window contents.
    Fire,
    /// Discard contents without computing.
    Purge,
    /// Compute result, then discard contents.
    FireAndPurge,
}

/// Decides when to compute a window's result.
pub trait Trigger: Send {
    fn on_element(&mut self, timestamp: EventTimestamp, window: &Window) -> TriggerResult;
    fn on_event_time(&mut self, watermark: EventTimestamp, window: &Window) -> TriggerResult;
}

/// Fires when the watermark passes the window's end timestamp. Default trigger.
#[derive(Debug, Clone)]
pub struct EventTimeTrigger;

impl Trigger for EventTimeTrigger {
    fn on_element(&mut self, _timestamp: EventTimestamp, _window: &Window) -> TriggerResult {
        TriggerResult::Continue
    }

    fn on_event_time(&mut self, watermark: EventTimestamp, window: &Window) -> TriggerResult {
        if watermark >= window.end {
            TriggerResult::FireAndPurge
        } else {
            TriggerResult::Continue
        }
    }
}

// ---------------------------------------------------------------------------
// Aggregate Function
// ---------------------------------------------------------------------------

/// Core abstraction for window computations with incremental aggregation.
pub trait AggregateFunction<In, Acc, Out>: Send {
    /// Create a fresh accumulator.
    fn create_accumulator(&self) -> Acc;
    /// Add a value to the accumulator.
    fn add(&self, acc: &mut Acc, value: &In);
    /// Extract the result from the accumulator.
    fn get_result(&self, acc: &Acc) -> Out;
    /// Merge two accumulators (for combining partitions).
    fn merge(&self, a: &mut Acc, b: Acc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_contains() {
        let w = Window::new(EventTimestamp::new(100), EventTimestamp::new(200));
        assert!(w.contains(EventTimestamp::new(100)));
        assert!(w.contains(EventTimestamp::new(150)));
        assert!(!w.contains(EventTimestamp::new(200))); // exclusive end
        assert!(!w.contains(EventTimestamp::new(99)));
    }

    #[test]
    fn test_tumbling_window() {
        let assigner = TumblingWindow::new(Duration::from_secs(60));
        let windows = assigner.assign_windows(EventTimestamp::new(75_000));
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].start, EventTimestamp::new(60_000));
        assert_eq!(windows[0].end, EventTimestamp::new(120_000));
    }

    #[test]
    fn test_tumbling_window_boundary() {
        let assigner = TumblingWindow::new(Duration::from_secs(10));
        // Exactly on boundary → belongs to the next window
        let windows = assigner.assign_windows(EventTimestamp::new(10_000));
        assert_eq!(windows[0].start, EventTimestamp::new(10_000));
        assert_eq!(windows[0].end, EventTimestamp::new(20_000));
    }

    #[test]
    fn test_sliding_window() {
        let assigner = SlidingWindow::new(Duration::from_secs(60), Duration::from_secs(10));
        let windows = assigner.assign_windows(EventTimestamp::new(25_000));
        // 25s should belong to windows starting at: 0, 10, 20 (all containing 25s)
        // Window [0, 60) contains 25s ✓
        // Window [10, 70) contains 25s ✓
        // Window [20, 80) contains 25s ✓
        assert!(windows.len() >= 3);
        for w in &windows {
            assert!(w.contains(EventTimestamp::new(25_000)));
        }
    }

    #[test]
    fn test_event_time_trigger() {
        let mut trigger = EventTimeTrigger;
        let window = Window::new(EventTimestamp::new(0), EventTimestamp::new(60_000));

        // Watermark before window end → continue
        assert_eq!(
            trigger.on_event_time(EventTimestamp::new(30_000), &window),
            TriggerResult::Continue
        );

        // Watermark at window end → fire and purge
        assert_eq!(
            trigger.on_event_time(EventTimestamp::new(60_000), &window),
            TriggerResult::FireAndPurge
        );

        // Watermark past window end → fire and purge
        assert_eq!(
            trigger.on_event_time(EventTimestamp::new(90_000), &window),
            TriggerResult::FireAndPurge
        );
    }
}
