//! Task scheduler — spawns and manages operator task lifecycles.

use flume_core::{FlumeError, FlumeResult};
use metrics::{counter, gauge};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

/// Manages spawned async tasks and their lifecycle.
pub struct Scheduler {
    handles: Vec<(String, JoinHandle<FlumeResult<()>>)>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    /// Spawn a named async task.
    pub fn spawn(
        &mut self,
        name: impl Into<String>,
        future: impl std::future::Future<Output = FlumeResult<()>> + Send + 'static,
    ) {
        let name = name.into();
        info!(task = %name, "spawning task");
        counter!("flume.tasks.spawned").increment(1);
        gauge!("flume.tasks.active").increment(1.0);
        let handle = tokio::spawn(future);
        self.handles.push((name, handle));
    }

    /// Spawn a named task pinned to a CPU core (requires `perf` feature).
    ///
    /// Falls back to `tokio::spawn` when the `perf` feature is disabled.
    pub fn spawn_pinned(
        &mut self,
        name: impl Into<String>,
        core_id: usize,
        future: impl std::future::Future<Output = FlumeResult<()>> + Send + 'static,
    ) {
        let name = name.into();
        let handle = crate::pinned::spawn_pinned(core_id, future);
        self.handles.push((name, handle));
    }

    /// Wait for all tasks to complete. Returns the first error encountered.
    pub async fn wait_all(self) -> FlumeResult<()> {
        let mut first_error: Option<FlumeError> = None;

        for (name, handle) in self.handles {
            match handle.await {
                Ok(Ok(())) => {
                    gauge!("flume.tasks.active").decrement(1.0);
                    debug!(task = %name, "task completed successfully");
                }
                Ok(Err(e)) => {
                    gauge!("flume.tasks.active").decrement(1.0);
                    counter!("flume.tasks.failed").increment(1);
                    error!(task = %name, error = %e, "task failed");
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
                Err(join_err) => {
                    gauge!("flume.tasks.active").decrement(1.0);
                    counter!("flume.tasks.failed").increment(1);
                    error!(task = %name, error = %join_err, "task panicked");
                    if first_error.is_none() {
                        first_error = Some(FlumeError::Execution(format!(
                            "task '{name}' panicked: {join_err}"
                        )));
                    }
                }
            }
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_all_succeed() {
        let mut scheduler = Scheduler::new();
        scheduler.spawn("task-1", async { Ok(()) });
        scheduler.spawn("task-2", async { Ok(()) });

        scheduler.wait_all().await.unwrap();
    }

    #[tokio::test]
    async fn test_task_error_propagates() {
        let mut scheduler = Scheduler::new();
        scheduler.spawn("ok", async { Ok(()) });
        scheduler.spawn("fail", async { Err(FlumeError::Execution("boom".into())) });

        let result = scheduler.wait_all().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));
    }

    #[tokio::test]
    async fn test_panic_is_caught() {
        let mut scheduler = Scheduler::new();
        scheduler.spawn("panic", async {
            panic!("test panic");
        });

        let result = scheduler.wait_all().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("panicked"));
    }
}
