//! Server configuration loading from TOML with environment variable overlay.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use flume_core::{FlumeError, FlumeResult, PipelineConfig};
use serde::{Deserialize, Serialize};

/// Server-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address to bind the HTTP server to.
    #[serde(default = "default_bind_addr")]
    pub bind_addr: SocketAddr,

    /// Pipeline configuration shared across jobs.
    #[serde(default)]
    pub pipeline: PipelineConfig,

    /// Directory for checkpoint storage.
    #[serde(default = "default_checkpoint_dir")]
    pub checkpoint_dir: PathBuf,
}

fn default_bind_addr() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}

fn default_checkpoint_dir() -> PathBuf {
    PathBuf::from("/tmp/flume/checkpoints")
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            pipeline: PipelineConfig::default(),
            checkpoint_dir: default_checkpoint_dir(),
        }
    }
}

/// Load server config from an optional TOML file, with env var overlay.
pub fn load_server_config(toml_path: Option<&Path>) -> FlumeResult<ServerConfig> {
    let mut config = match toml_path {
        Some(path) => {
            let contents = std::fs::read_to_string(path).map_err(|e| {
                FlumeError::Config(format!("failed to read {}: {e}", path.display()))
            })?;
            toml::from_str(&contents)
                .map_err(|e| FlumeError::Config(format!("failed to parse TOML: {e}")))?
        }
        None => ServerConfig::default(),
    };

    // Apply env var overrides for server-level settings.
    if let Ok(val) = std::env::var("FLUME_BIND_ADDR") {
        config.bind_addr = val
            .parse()
            .map_err(|e| FlumeError::Config(format!("invalid FLUME_BIND_ADDR: {e}")))?;
    }
    if let Ok(val) = std::env::var("FLUME_CHECKPOINT_DIR") {
        config.checkpoint_dir = PathBuf::from(val);
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let config = ServerConfig::default();
        assert_eq!(config.bind_addr, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(
            config.checkpoint_dir,
            PathBuf::from("/tmp/flume/checkpoints")
        );
    }

    #[test]
    fn test_load_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        std::fs::write(
            &path,
            r#"
bind_addr = "127.0.0.1:9090"
checkpoint_dir = "/data/checkpoints"

[pipeline]
job_name = "test-job"
parallelism = 4
"#,
        )
        .unwrap();

        let config = load_server_config(Some(&path)).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1:9090".parse().unwrap());
        assert_eq!(config.checkpoint_dir, PathBuf::from("/data/checkpoints"));
        assert_eq!(config.pipeline.job_name, "test-job");
        assert_eq!(config.pipeline.parallelism, 4);
    }
}
