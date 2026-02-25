//! HTTP handler functions for the REST API.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use uuid::Uuid;

use super::error::ApiError;
use super::models::{
    CancelJobResponse, HealthResponse, JobDetailResponse, JobListResponse, JobSummary,
    SubmitJobRequest, SubmitJobResponse, TaskManagerListResponse, TaskManagerSummary,
};
use super::state::ServerState;
use crate::job::{JobHandle, JobStatus};

/// POST /api/v1/jobs — submit a new job.
pub async fn submit_job(
    State(state): State<ServerState>,
    Json(req): Json<SubmitJobRequest>,
) -> Result<(StatusCode, Json<SubmitJobResponse>), ApiError> {
    let mut inner = state.lock().await;

    let factory = inner
        .registry
        .get(&req.job_name)
        .ok_or_else(|| ApiError::not_found(format!("unknown job: {}", req.job_name)))?
        .clone();

    let mut config = inner.config.clone();
    config.job_name = req.job_name.clone();
    if let Some(p) = req.parallelism {
        config.parallelism = p;
    }
    if let Some(b) = req.channel_buffer_size {
        config.channel_buffer_size = b;
    }

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let future = factory(config, cancel_clone);
    let task_handle = tokio::spawn(future);

    let handle = JobHandle::new(req.job_name.clone(), cancel, task_handle);
    let id = handle.id;
    info!(job_id = %id, job_name = %req.job_name, "job submitted");

    let response = SubmitJobResponse {
        job_id: id,
        job_name: req.job_name,
        status: handle.status.clone(),
    };

    inner.jobs.insert(id, handle);

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/v1/jobs — list all jobs.
pub async fn list_jobs(State(state): State<ServerState>) -> Json<JobListResponse> {
    let inner = state.lock().await;
    let jobs = inner
        .jobs
        .values()
        .map(|h| JobSummary {
            job_id: h.id,
            job_name: h.name.clone(),
            status: h.status.clone(),
        })
        .collect();
    Json(JobListResponse { jobs })
}

/// GET /api/v1/jobs/:id — get a single job's details.
pub async fn get_job(
    State(state): State<ServerState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobDetailResponse>, ApiError> {
    let inner = state.lock().await;
    let handle = inner
        .jobs
        .get(&id)
        .ok_or_else(|| ApiError::not_found(format!("job not found: {id}")))?;

    let uptime = if !handle.is_terminal() {
        Some(handle.uptime_secs())
    } else {
        None
    };

    Ok(Json(JobDetailResponse {
        job_id: handle.id,
        job_name: handle.name.clone(),
        status: handle.status.clone(),
        uptime_secs: uptime,
    }))
}

/// POST /api/v1/jobs/:id/cancel — cancel a running job.
pub async fn cancel_job(
    State(state): State<ServerState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CancelJobResponse>, ApiError> {
    let mut inner = state.lock().await;
    let handle = inner
        .jobs
        .get_mut(&id)
        .ok_or_else(|| ApiError::not_found(format!("job not found: {id}")))?;

    if handle.is_terminal() {
        return Err(ApiError::conflict(format!(
            "job {id} already in terminal state: {}",
            handle.status
        )));
    }

    info!(job_id = %id, "cancelling job");
    handle.cancel_token.cancel();
    handle.status = JobStatus::Cancelling;

    Ok(Json(CancelJobResponse {
        job_id: id,
        status: JobStatus::Cancelling,
    }))
}

/// GET /health/live — liveness probe.
pub async fn liveness() -> StatusCode {
    StatusCode::OK
}

/// GET /health/ready — readiness probe with stats.
pub async fn readiness(State(state): State<ServerState>) -> Json<HealthResponse> {
    let inner = state.lock().await;
    let active = inner.jobs.values().filter(|h| !h.is_terminal()).count();
    Json(HealthResponse {
        status: "ready".to_string(),
        registered_jobs: inner.registry.len(),
        active_jobs: active,
    })
}

/// GET /api/v1/taskmanagers — list registered TaskManagers.
pub async fn list_taskmanagers(
    State(state): State<ServerState>,
) -> Result<Json<TaskManagerListResponse>, ApiError> {
    let inner = state.lock().await;
    let rm = inner
        .resource_manager
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("not running in JM mode"))?;

    let rm_guard = rm.lock().await;
    let task_managers = rm_guard
        .list()
        .iter()
        .map(|info| TaskManagerSummary {
            id: info.id.clone(),
            address: info.address.clone(),
            total_slots: info.num_slots as usize,
            used_slots: info.slots_in_use as usize,
            registered_jobs: info.registered_jobs.clone(),
        })
        .collect();

    Ok(Json(TaskManagerListResponse { task_managers }))
}

/// Background task that polls finished job handles and updates their status.
pub async fn job_reaper(state: ServerState) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let mut inner = state.lock().await;
        for handle in inner.jobs.values_mut() {
            if handle.task_handle.is_finished() && !handle.is_terminal() {
                // The task is done — determine final status.
                // We can't await the JoinHandle without ownership, so use
                // the cancellation token to infer cancelled vs completed.
                if handle.status == JobStatus::Cancelling {
                    handle.status = JobStatus::Cancelled;
                    info!(job_id = %handle.id, "job cancelled");
                } else {
                    // Need to take ownership of the handle to get the result.
                    // We'll use a placeholder to swap it out.
                    // Since is_finished() is true, we can poll it.
                    // Unfortunately JoinHandle doesn't have try_join, so we
                    // check the cancel token as a heuristic.
                    if handle.cancel_token.is_cancelled() {
                        handle.status = JobStatus::Cancelled;
                        info!(job_id = %handle.id, "job cancelled");
                    } else {
                        handle.status = JobStatus::Completed;
                        info!(job_id = %handle.id, "job completed");
                    }
                }
            }
        }
    }
}

/// Spawn the job reaper as a background task, using a separate strategy
/// that takes ownership of finished handles to get actual results.
pub fn spawn_job_reaper(state: ServerState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            reap_finished_jobs(&state).await;
        }
    })
}

async fn reap_finished_jobs(state: &ServerState) {
    let mut inner = state.lock().await;
    let finished_ids: Vec<Uuid> = inner
        .jobs
        .iter()
        .filter(|(_, h)| h.task_handle.is_finished() && !h.is_terminal())
        .map(|(id, _)| *id)
        .collect();

    for id in finished_ids {
        // Take ownership of the handle to await it.
        let mut handle = inner.jobs.remove(&id).unwrap();
        let result = (&mut handle.task_handle).await;
        handle.status = match result {
            Ok(Ok(())) => {
                if handle.cancel_token.is_cancelled() {
                    info!(job_id = %id, "job cancelled");
                    JobStatus::Cancelled
                } else {
                    info!(job_id = %id, "job completed");
                    JobStatus::Completed
                }
            }
            Ok(Err(e)) => {
                error!(job_id = %id, error = %e, "job failed");
                JobStatus::Failed(e.to_string())
            }
            Err(join_err) => {
                error!(job_id = %id, error = %join_err, "job panicked");
                JobStatus::Failed(format!("panic: {join_err}"))
            }
        };
        // Re-insert with updated status.
        inner.jobs.insert(id, handle);
    }
}
