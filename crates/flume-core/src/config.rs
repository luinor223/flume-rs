//! Global pipeline configuration.

use std::time::Duration;

/// Global settings for a pipeline execution.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Human-readable job name.
    pub job_name: String,
    /// Default parallelism for operators.
    pub parallelism: usize,
    /// Bounded channel buffer size between operators.
    pub channel_buffer_size: usize,
    /// Checkpoint interval. `None` disables automatic checkpointing.
    pub checkpoint_interval: Option<Duration>,
    /// How long past the watermark a late record is still accepted.
    pub allowed_lateness: Duration,
    /// How long to wait for all operators to acknowledge a checkpoint barrier.
    pub checkpoint_timeout: Duration,
    /// Number of completed checkpoints to retain before cleaning up old ones.
    pub max_retained_checkpoints: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            job_name: "flume-job".into(),
            parallelism: 1,
            channel_buffer_size: 1024,
            checkpoint_interval: None,
            allowed_lateness: Duration::ZERO,
            checkpoint_timeout: Duration::from_secs(60),
            max_retained_checkpoints: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = PipelineConfig::default();
        assert_eq!(config.job_name, "flume-job");
        assert_eq!(config.parallelism, 1);
        assert_eq!(config.channel_buffer_size, 1024);
        assert!(config.checkpoint_interval.is_none());
        assert_eq!(config.allowed_lateness, Duration::ZERO);
        assert_eq!(config.checkpoint_timeout, Duration::from_secs(60));
        assert_eq!(config.max_retained_checkpoints, 3);
    }
}
