//! Unified error types for the flume-rs engine.

use std::error::Error;

use thiserror::Error;

/// Unified error type covering all flume-rs subsystems.
#[derive(Debug, Error)]
pub enum FlumeError {
    /// Source connector I/O errors.
    #[error("source error: {0}")]
    Source(Box<dyn Error + Send + Sync>),

    /// Sink connector I/O errors.
    #[error("sink error: {0}")]
    Sink(Box<dyn Error + Send + Sync>),

    /// Operator logic errors.
    #[error("operator error: {0}")]
    Operator(String),

    /// Serialization/deserialization errors.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// State backend errors.
    #[error("state error: {0}")]
    State(String),

    /// Checkpoint errors.
    #[error("checkpoint error: {0}")]
    Checkpoint(String),

    /// Downstream channel was dropped.
    #[error("channel closed")]
    ChannelClosed,

    /// Invalid pipeline configuration.
    #[error("config error: {0}")]
    Config(String),

    /// Runtime execution errors.
    #[error("execution error: {0}")]
    Execution(String),

    /// General I/O errors.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias used throughout the crate.
pub type FlumeResult<T> = Result<T, FlumeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        let err = FlumeError::Operator("bad input".into());
        assert_eq!(err.to_string(), "operator error: bad input");
    }

    #[test]
    fn test_io_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err = FlumeError::from(io_err);
        assert!(matches!(err, FlumeError::Io(_)));
    }

    #[test]
    fn test_channel_closed() {
        let err = FlumeError::ChannelClosed;
        assert_eq!(err.to_string(), "channel closed");
    }
}
