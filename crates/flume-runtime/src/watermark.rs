//! Multi-input watermark tracking and propagation.

use flume_core::{EventTimestamp, Watermark};

/// Tracks watermarks across multiple input channels and emits the minimum.
///
/// The output watermark is `min(all input watermarks)`. This guarantees
/// correctness: the watermark promise ("no records <= this timestamp will
/// arrive") must hold across all inputs.
pub struct WatermarkTracker {
    /// One watermark per input channel.
    input_watermarks: Vec<EventTimestamp>,
    /// Last emitted output watermark.
    current_watermark: EventTimestamp,
}

impl WatermarkTracker {
    /// Create a tracker for `num_inputs` input channels.
    pub fn new(num_inputs: usize) -> Self {
        Self {
            input_watermarks: vec![EventTimestamp::MIN; num_inputs],
            current_watermark: EventTimestamp::MIN,
        }
    }

    /// Update the watermark for a specific input channel.
    ///
    /// Returns `Some(watermark)` if the output watermark advanced.
    pub fn update(&mut self, input_index: usize, watermark: Watermark) -> Option<Watermark> {
        self.input_watermarks[input_index] = watermark.timestamp;

        let new_min = self
            .input_watermarks
            .iter()
            .copied()
            .min()
            .unwrap_or(EventTimestamp::MIN);

        if new_min > self.current_watermark {
            self.current_watermark = new_min;
            Some(Watermark::new(new_min))
        } else {
            None
        }
    }

    /// Returns the current output watermark.
    pub fn current(&self) -> EventTimestamp {
        self.current_watermark
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_input() {
        let mut tracker = WatermarkTracker::new(1);

        let result = tracker.update(0, Watermark::new(EventTimestamp::new(100)));
        assert!(result.is_some());
        assert_eq!(tracker.current(), EventTimestamp::new(100));
    }

    #[test]
    fn test_multi_input_takes_min() {
        let mut tracker = WatermarkTracker::new(2);

        // Input 0 advances to 200
        let result = tracker.update(0, Watermark::new(EventTimestamp::new(200)));
        assert!(result.is_none()); // Input 1 still at MIN

        // Input 1 advances to 100 → min is 100
        let result = tracker.update(1, Watermark::new(EventTimestamp::new(100)));
        assert_eq!(result.unwrap().timestamp, EventTimestamp::new(100));

        // Input 1 advances to 300 → min is now 200
        let result = tracker.update(1, Watermark::new(EventTimestamp::new(300)));
        assert_eq!(result.unwrap().timestamp, EventTimestamp::new(200));
    }

    #[test]
    fn test_watermark_monotonicity() {
        let mut tracker = WatermarkTracker::new(1);

        tracker.update(0, Watermark::new(EventTimestamp::new(100)));

        // Same watermark → no advance
        let result = tracker.update(0, Watermark::new(EventTimestamp::new(100)));
        assert!(result.is_none());

        // Lower watermark → no advance (monotonicity)
        let result = tracker.update(0, Watermark::new(EventTimestamp::new(50)));
        assert!(result.is_none());
        assert_eq!(tracker.current(), EventTimestamp::new(100));
    }
}
