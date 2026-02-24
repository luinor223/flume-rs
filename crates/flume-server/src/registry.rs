//! Job registry — compile-time registration of named job factories.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use flume_core::{FlumeResult, PipelineConfig};
use tokio_util::sync::CancellationToken;

/// A boxed future returned by a job factory.
pub type BoxJobFuture = Pin<Box<dyn Future<Output = FlumeResult<()>> + Send>>;

/// A factory function that creates a job pipeline from config and a cancellation token.
pub type JobFactory = Arc<dyn Fn(PipelineConfig, CancellationToken) -> BoxJobFuture + Send + Sync>;

/// Registry of named job factories.
///
/// Users register job factories at compile time. The server binary includes all
/// registered jobs and starts/stops them by name via the REST API.
pub struct JobRegistry {
    factories: HashMap<String, JobFactory>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a named job factory.
    pub fn register<F, Fut>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn(PipelineConfig, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = FlumeResult<()>> + Send + 'static,
    {
        let factory = Arc::new(move |config, cancel| {
            Box::pin(factory(config, cancel)) as BoxJobFuture
        });
        self.factories.insert(name.into(), factory);
    }

    /// Look up a factory by name.
    pub fn get(&self, name: &str) -> Option<&JobFactory> {
        self.factories.get(name)
    }

    /// Return a sorted list of registered job names.
    pub fn list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.factories.keys().cloned().collect();
        names.sort();
        names
    }

    /// Return the number of registered factories.
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Return true if no factories are registered.
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Default for JobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_get() {
        let mut registry = JobRegistry::new();
        registry.register("my-job", |_config, _cancel| async { Ok(()) });

        assert!(registry.get("my-job").is_some());
    }

    #[test]
    fn test_unknown_returns_none() {
        let registry = JobRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_list_names_sorted() {
        let mut registry = JobRegistry::new();
        registry.register("zebra", |_config, _cancel| async { Ok(()) });
        registry.register("alpha", |_config, _cancel| async { Ok(()) });
        registry.register("middle", |_config, _cancel| async { Ok(()) });

        assert_eq!(registry.list_names(), vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut registry = JobRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry.register("job-1", |_config, _cancel| async { Ok(()) });
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }
}
