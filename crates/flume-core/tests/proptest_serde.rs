//! Property-based tests for PipelineConfig TOML serialization roundtrips.

use std::time::Duration;

use flume_core::{ChannelKind, PipelineConfig};
use proptest::prelude::*;

fn arb_channel_kind() -> impl Strategy<Value = ChannelKind> {
    prop_oneof![Just(ChannelKind::Mpsc), Just(ChannelKind::RingBuffer),]
}

fn arb_pipeline_config() -> impl Strategy<Value = PipelineConfig> {
    (
        "[a-z]{1,20}",                        // job_name
        1..=64usize,                          // parallelism
        1..=65536usize,                       // channel_buffer_size
        proptest::option::of(1..=600_000u64), // checkpoint_interval_ms
        0..=60_000u64,                        // allowed_lateness_ms
        1..=600_000u64,                       // checkpoint_timeout_ms
        1..=100usize,                         // max_retained_checkpoints
        arb_channel_kind(),
    )
        .prop_map(
            |(
                job_name,
                parallelism,
                channel_buffer_size,
                checkpoint_interval_ms,
                allowed_lateness_ms,
                checkpoint_timeout_ms,
                max_retained_checkpoints,
                channel_kind,
            )| {
                PipelineConfig {
                    job_name,
                    parallelism,
                    channel_buffer_size,
                    checkpoint_interval: checkpoint_interval_ms.map(Duration::from_millis),
                    allowed_lateness: Duration::from_millis(allowed_lateness_ms),
                    checkpoint_timeout: Duration::from_millis(checkpoint_timeout_ms),
                    max_retained_checkpoints,
                    channel_kind,
                }
            },
        )
}

proptest! {
    #[test]
    fn toml_roundtrip(config in arb_pipeline_config()) {
        let toml_str = toml::to_string(&config).unwrap();
        let deserialized: PipelineConfig = toml::from_str(&toml_str).unwrap();
        prop_assert_eq!(config, deserialized);
    }
}
