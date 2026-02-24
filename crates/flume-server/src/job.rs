//! Job handle and status tracking.

use std::time::Instant;

use flume_core::FlumeResult;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Status of a running or completed job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed(String),
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Running => write!(f, "running"),
            JobStatus::Cancelling => write!(f, "cancelling"),
            JobStatus::Cancelled => write!(f, "cancelled"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

/// A running job instance tracked by the server.
pub struct JobHandle {
    pub id: Uuid,
    pub name: String,
    pub status: JobStatus,
    pub started_at: Instant,
    pub cancel_token: CancellationToken,
    pub task_handle: JoinHandle<FlumeResult<()>>,
}

impl JobHandle {
    /// Create a new running job handle.
    pub fn new(
        name: String,
        cancel_token: CancellationToken,
        task_handle: JoinHandle<FlumeResult<()>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            status: JobStatus::Running,
            started_at: Instant::now(),
            cancel_token,
            task_handle,
        }
    }

    /// Return uptime in seconds since the job started.
    pub fn uptime_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    /// Return true if the job has reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed(_)
        )
    }
}
