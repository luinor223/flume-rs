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
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            job_name: "flume-job".into(),
            parallelism: 1,
            channel_buffer_size: 1024,
            checkpoint_interval: None,
            allowed_lateness: Duration::ZERO,
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
    }
}
