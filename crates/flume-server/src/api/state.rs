//! Shared server state behind a `tokio::sync::Mutex`.

use std::collections::HashMap;
use std::sync::Arc;

use flume_core::PipelineConfig;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::job::JobHandle;
use crate::registry::JobRegistry;

/// Shared server state accessible from all handlers.
pub type ServerState = Arc<Mutex<ServerInner>>;

/// Inner server state protected by a mutex.
pub struct ServerInner {
    pub registry: JobRegistry,
    pub jobs: HashMap<Uuid, JobHandle>,
    pub config: PipelineConfig,
}

impl ServerInner {
    pub fn new(registry: JobRegistry, config: PipelineConfig) -> Self {
        Self {
            registry,
            jobs: HashMap::new(),
            config,
        }
    }
}

/// Create a new `ServerState` wrapping the given registry and config.
pub fn new_state(registry: JobRegistry, config: PipelineConfig) -> ServerState {
    Arc::new(Mutex::new(ServerInner::new(registry, config)))
}
