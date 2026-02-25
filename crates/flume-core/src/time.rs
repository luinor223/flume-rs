//! Event-time timestamp type for stream processing.

use std::fmt;

/// Millisecond-precision timestamp representing when an event occurred in the real world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct EventTimestamp(pub u64);

impl EventTimestamp {
    /// No progress yet.
    pub const MIN: Self = Self(0);

    /// End-of-stream marker.
    pub const MAX: Self = Self(u64::MAX);

    /// Create a new timestamp from milliseconds since UNIX epoch.
    pub fn new(millis: u64) -> Self {
        Self(millis)
    }

    /// Returns the inner millisecond value.
    pub fn as_millis(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EventTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventTimestamp({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordering() {
        let t1 = EventTimestamp::new(100);
        let t2 = EventTimestamp::new(200);
        assert!(t1 < t2);
    }

    #[test]
    fn test_min_max() {
        assert_eq!(EventTimestamp::MIN.as_millis(), 0);
        assert_eq!(EventTimestamp::MAX.as_millis(), u64::MAX);
    }

    #[test]
    fn test_equality() {
        let t1 = EventTimestamp::new(42);
        let t2 = EventTimestamp::new(42);
        assert_eq!(t1, t2);
    }
}
