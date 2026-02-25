//! JSON request/response models for the REST API.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::job::JobStatus;

/// Request body for submitting a new job.
#[derive(Debug, Deserialize)]
pub struct SubmitJobRequest {
    pub job_name: String,
    pub parallelism: Option<usize>,
    pub channel_buffer_size: Option<usize>,
}

/// Response body after submitting a job.
#[derive(Debug, Serialize)]
pub struct SubmitJobResponse {
    pub job_id: Uuid,
    pub job_name: String,
    pub status: JobStatus,
}

/// Detailed response for a single job.
#[derive(Debug, Serialize)]
pub struct JobDetailResponse {
    pub job_id: Uuid,
    pub job_name: String,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_secs: Option<f64>,
}

/// Summary of a job for list responses.
#[derive(Debug, Serialize)]
pub struct JobSummary {
    pub job_id: Uuid,
    pub job_name: String,
    pub status: JobStatus,
}

/// Response body for listing jobs.
#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobSummary>,
}

/// Response body after cancelling a job.
#[derive(Debug, Serialize)]
pub struct CancelJobResponse {
    pub job_id: Uuid,
    pub status: JobStatus,
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub registered_jobs: usize,
    pub active_jobs: usize,
}
