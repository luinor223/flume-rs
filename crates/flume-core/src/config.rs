//! Global pipeline configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Which channel implementation to use between operators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    /// `tokio::sync::mpsc` bounded channel (default, general-purpose).
    #[default]
    Mpsc,
    /// Lock-free SPSC ring buffer (lower latency for single-producer edges).
    RingBuffer,
}

/// Global settings for a pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineConfig {
    /// Human-readable job name.
    #[serde(default = "default_job_name")]
    pub job_name: String,
    /// Default parallelism for operators.
    #[serde(default = "default_parallelism")]
    pub parallelism: usize,
    /// Bounded channel buffer size between operators.
    #[serde(default = "default_channel_buffer_size")]
    pub channel_buffer_size: usize,
    /// Checkpoint interval in milliseconds. `None` disables automatic checkpointing.
    #[serde(
        default,
        with = "duration_millis_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub checkpoint_interval: Option<Duration>,
    /// How long past the watermark a late record is still accepted (milliseconds).
    #[serde(default, with = "duration_millis")]
    pub allowed_lateness: Duration,
    /// How long to wait for all operators to acknowledge a checkpoint barrier (milliseconds).
    #[serde(default = "default_checkpoint_timeout", with = "duration_millis")]
    pub checkpoint_timeout: Duration,
    /// Number of completed checkpoints to retain before cleaning up old ones.
    #[serde(default = "default_max_retained_checkpoints")]
    pub max_retained_checkpoints: usize,
    /// Channel implementation between operators.
    #[serde(default)]
    pub channel_kind: ChannelKind,
}

fn default_job_name() -> String {
    "flume-job".into()
}
fn default_parallelism() -> usize {
    1
}
fn default_channel_buffer_size() -> usize {
    1024
}
fn default_checkpoint_timeout() -> Duration {
    Duration::from_secs(60)
}
fn default_max_retained_checkpoints() -> usize {
    3
}

/// Serde module for Duration as u64 milliseconds.
pub mod duration_millis {
    use std::time::Duration;

    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

/// Serde module for `Option<Duration>` as optional u64 milliseconds.
mod duration_millis_opt {
    use std::time::Duration;

    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => serializer.serialize_some(&(d.as_millis() as u64)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            job_name: default_job_name(),
            parallelism: default_parallelism(),
            channel_buffer_size: default_channel_buffer_size(),
            checkpoint_interval: None,
            allowed_lateness: Duration::ZERO,
            checkpoint_timeout: default_checkpoint_timeout(),
            max_retained_checkpoints: default_max_retained_checkpoints(),
            channel_kind: ChannelKind::default(),
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
