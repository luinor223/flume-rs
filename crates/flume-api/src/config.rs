//! TOML configuration loading with environment variable overlay.

use std::path::Path;

use flume_core::{FlumeError, FlumeResult, PipelineConfig};

/// Load a [`PipelineConfig`] from an optional TOML file path, then overlay
/// environment variables on top.
///
/// # Environment variables
///
/// | Variable                          | Field                    |
/// |-----------------------------------|--------------------------|
/// | `FLUME_JOB_NAME`                  | `job_name`               |
/// | `FLUME_PARALLELISM`               | `parallelism`            |
/// | `FLUME_CHANNEL_BUFFER_SIZE`       | `channel_buffer_size`    |
/// | `FLUME_CHECKPOINT_INTERVAL_MS`    | `checkpoint_interval`    |
/// | `FLUME_ALLOWED_LATENESS_MS`       | `allowed_lateness`       |
/// | `FLUME_CHECKPOINT_TIMEOUT_MS`     | `checkpoint_timeout`     |
/// | `FLUME_MAX_RETAINED_CHECKPOINTS`  | `max_retained_checkpoints` |
/// | `FLUME_CHANNEL_KIND`              | `channel_kind`           |
pub fn load_config(toml_path: Option<&Path>) -> FlumeResult<PipelineConfig> {
    let mut config = load_toml(toml_path)?;
    apply_env_overlay(&mut config, |key| std::env::var(key))?;
    Ok(config)
}

fn load_toml(toml_path: Option<&Path>) -> FlumeResult<PipelineConfig> {
    match toml_path {
        Some(path) => {
            let contents = std::fs::read_to_string(path).map_err(|e| {
                FlumeError::Config(format!("failed to read {}: {e}", path.display()))
            })?;
            toml::from_str(&contents)
                .map_err(|e| FlumeError::Config(format!("failed to parse TOML: {e}")))
        }
        None => Ok(PipelineConfig::default()),
    }
}

fn apply_env_overlay(
    config: &mut PipelineConfig,
    get_var: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> FlumeResult<()> {
    if let Ok(val) = get_var("FLUME_JOB_NAME") {
        config.job_name = val;
    }
    if let Ok(val) = get_var("FLUME_PARALLELISM") {
        config.parallelism = parse_env("FLUME_PARALLELISM", &val)?;
    }
    if let Ok(val) = get_var("FLUME_CHANNEL_BUFFER_SIZE") {
        config.channel_buffer_size = parse_env("FLUME_CHANNEL_BUFFER_SIZE", &val)?;
    }
    if let Ok(val) = get_var("FLUME_CHECKPOINT_INTERVAL_MS") {
        let ms: u64 = parse_env("FLUME_CHECKPOINT_INTERVAL_MS", &val)?;
        config.checkpoint_interval = Some(std::time::Duration::from_millis(ms));
    }
    if let Ok(val) = get_var("FLUME_ALLOWED_LATENESS_MS") {
        let ms: u64 = parse_env("FLUME_ALLOWED_LATENESS_MS", &val)?;
        config.allowed_lateness = std::time::Duration::from_millis(ms);
    }
    if let Ok(val) = get_var("FLUME_CHECKPOINT_TIMEOUT_MS") {
        let ms: u64 = parse_env("FLUME_CHECKPOINT_TIMEOUT_MS", &val)?;
        config.checkpoint_timeout = std::time::Duration::from_millis(ms);
    }
    if let Ok(val) = get_var("FLUME_MAX_RETAINED_CHECKPOINTS") {
        config.max_retained_checkpoints = parse_env("FLUME_MAX_RETAINED_CHECKPOINTS", &val)?;
    }
    if let Ok(val) = get_var("FLUME_CHANNEL_KIND") {
        config.channel_kind = match val.as_str() {
            "mpsc" => flume_core::ChannelKind::Mpsc,
            "ring_buffer" => flume_core::ChannelKind::RingBuffer,
            other => {
                return Err(FlumeError::Config(format!(
                    "invalid FLUME_CHANNEL_KIND: '{other}' (expected 'mpsc' or 'ring_buffer')"
                )));
            }
        };
    }
    Ok(())
}

fn parse_env<T: std::str::FromStr>(name: &str, val: &str) -> FlumeResult<T>
where
    T::Err: std::fmt::Display,
{
    val.parse()
        .map_err(|e| FlumeError::Config(format!("invalid {name}: {e}")))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use flume_core::ChannelKind;

    use super::*;

    /// Build a fake env lookup from a map.
    fn fake_env(vars: HashMap<&str, &str>) -> impl Fn(&str) -> Result<String, std::env::VarError> {
        let owned: HashMap<String, String> = vars
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        move |key: &str| {
            owned
                .get(key)
                .cloned()
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    /// No-env lookup: all vars are absent.
    fn no_env(_key: &str) -> Result<String, std::env::VarError> {
        Err(std::env::VarError::NotPresent)
    }

    #[test]
    fn test_load_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flume.toml");
        std::fs::write(
            &path,
            r#"
job_name = "my-job"
parallelism = 4
channel_buffer_size = 2048
checkpoint_interval = 5000
allowed_lateness = 1000
checkpoint_timeout = 30000
max_retained_checkpoints = 5
channel_kind = "ring_buffer"
"#,
        )
        .unwrap();

        let mut config = load_toml(Some(&path)).unwrap();
        apply_env_overlay(&mut config, no_env).unwrap();

        assert_eq!(config.job_name, "my-job");
        assert_eq!(config.parallelism, 4);
        assert_eq!(config.channel_buffer_size, 2048);
        assert_eq!(
            config.checkpoint_interval,
            Some(Duration::from_millis(5000))
        );
        assert_eq!(config.allowed_lateness, Duration::from_millis(1000));
        assert_eq!(config.checkpoint_timeout, Duration::from_millis(30000));
        assert_eq!(config.max_retained_checkpoints, 5);
        assert_eq!(config.channel_kind, ChannelKind::RingBuffer);
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.toml");
        std::fs::write(
            &path,
            r#"
job_name = "partial-job"
"#,
        )
        .unwrap();

        let mut config = load_toml(Some(&path)).unwrap();
        apply_env_overlay(&mut config, no_env).unwrap();

        assert_eq!(config.job_name, "partial-job");
        assert_eq!(config.parallelism, 1);
        assert_eq!(config.channel_buffer_size, 1024);
        assert!(config.checkpoint_interval.is_none());
    }

    #[test]
    fn test_no_file_uses_defaults() {
        let mut config = load_toml(None).unwrap();
        apply_env_overlay(&mut config, no_env).unwrap();
        assert_eq!(config, PipelineConfig::default());
    }

    #[test]
    fn test_env_var_override() {
        let env = fake_env(HashMap::from([
            ("FLUME_JOB_NAME", "env-job"),
            ("FLUME_PARALLELISM", "8"),
        ]));

        let mut config = PipelineConfig::default();
        apply_env_overlay(&mut config, env).unwrap();

        assert_eq!(config.job_name, "env-job");
        assert_eq!(config.parallelism, 8);
    }

    #[test]
    fn test_env_over_toml_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("override.toml");
        std::fs::write(
            &path,
            r#"
job_name = "toml-job"
parallelism = 2
"#,
        )
        .unwrap();

        let env = fake_env(HashMap::from([("FLUME_JOB_NAME", "env-wins")]));

        let mut config = load_toml(Some(&path)).unwrap();
        apply_env_overlay(&mut config, env).unwrap();

        assert_eq!(config.job_name, "env-wins"); // env overrides TOML
        assert_eq!(config.parallelism, 2); // TOML value kept
    }

    #[test]
    fn test_invalid_env_var_errors() {
        let env = fake_env(HashMap::from([("FLUME_PARALLELISM", "not-a-number")]));

        let mut config = PipelineConfig::default();
        let result = apply_env_overlay(&mut config, env);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("FLUME_PARALLELISM")
        );
    }

    #[test]
    fn test_invalid_channel_kind_env() {
        let env = fake_env(HashMap::from([("FLUME_CHANNEL_KIND", "invalid")]));

        let mut config = PipelineConfig::default();
        let result = apply_env_overlay(&mut config, env);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("FLUME_CHANNEL_KIND")
        );
    }
}
