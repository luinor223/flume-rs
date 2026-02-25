//! Route wiring for the REST API.

use axum::Router;
use axum::routing::{get, post};

use super::handlers;
use super::state::ServerState;

/// Build the axum router with all API routes.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/api/v1/jobs", post(handlers::submit_job))
        .route("/api/v1/jobs", get(handlers::list_jobs))
        .route("/api/v1/jobs/{id}", get(handlers::get_job))
        .route("/api/v1/jobs/{id}/cancel", post(handlers::cancel_job))
        .route("/api/v1/taskmanagers", get(handlers::list_taskmanagers))
        .route("/health/live", get(handlers::liveness))
        .route("/health/ready", get(handlers::readiness))
        .with_state(state)
}
