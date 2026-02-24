//! CPU core pinning helpers for latency-sensitive tasks.
//!
//! When the `perf` feature is enabled, [`spawn_pinned`] pins the task to a
//! specific CPU core using a dedicated OS thread with a single-threaded tokio
//! runtime. When disabled, it falls back to `tokio::spawn`.

use flume_core::FlumeResult;
use tokio::task::JoinHandle;

/// Spawn an async task pinned to a specific CPU core.
///
/// With the `perf` feature, this creates a dedicated OS thread pinned to
/// `core_id` running a single-threaded tokio runtime. Without the feature,
/// it falls back to `tokio::spawn`.
#[cfg(feature = "perf")]
pub fn spawn_pinned(
    core_id: usize,
    future: impl std::future::Future<Output = FlumeResult<()>> + Send + 'static,
) -> JoinHandle<FlumeResult<()>> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        // Pin this thread to the requested core.
        let core_ids = core_affinity::get_core_ids().unwrap_or_default();
        if let Some(id) = core_ids.get(core_id % core_ids.len().max(1)) {
            core_affinity::set_for_current(*id);
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build pinned runtime");

        let result = rt.block_on(future);
        let _ = tx.send(result);
    });

    // Bridge the result back into the caller's tokio runtime.
    tokio::spawn(async move {
        rx.await.unwrap_or_else(|_| {
            Err(flume_core::FlumeError::Execution(
                "pinned task thread panicked".into(),
            ))
        })
    })
}

/// Spawn an async task (fallback: no core pinning).
#[cfg(not(feature = "perf"))]
pub fn spawn_pinned(
    _core_id: usize,
    future: impl std::future::Future<Output = FlumeResult<()>> + Send + 'static,
) -> JoinHandle<FlumeResult<()>> {
    tokio::spawn(future)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_pinned_completes() {
        let handle = spawn_pinned(0, async { Ok(()) });
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_spawn_pinned_returns_value() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();

        let handle = spawn_pinned(0, async move {
            flag_clone.store(true, Ordering::SeqCst);
            Ok(())
        });

        handle.await.unwrap().unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_spawn_pinned_propagates_error() {
        let handle = spawn_pinned(0, async {
            Err(flume_core::FlumeError::Execution("test error".into()))
        });

        let result = handle.await.unwrap();
        assert!(result.is_err());
    }
}
