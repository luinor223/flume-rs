//! Integration tests for the flume-server REST API.

use std::time::Duration;

use flume_api::StreamExecutionEnvironment;
use flume_core::PipelineConfig;
use flume_server::api::{handlers, routes, state};
use flume_server::registry::JobRegistry;
use tokio::net::TcpListener;

/// Start a test server on a random port and return the base URL.
async fn start_test_server() -> String {
    let mut registry = JobRegistry::new();

    // A fast job that completes immediately.
    registry.register("fast-job", |config, cancel| async move {
        let mut env = StreamExecutionEnvironment::from_config(config);
        env.connect_cancel_token(cancel);
        env.from_collection(vec![1, 2, 3])
            .map(|x| x * 2)
            .add_sink(flume_api::sink::CollectSink::<i32>::new().0)
            .await
    });

    // A long-running job that blocks until cancelled.
    registry.register("long-job", |_config, cancel| async move {
        cancel.cancelled().await;
        Ok(())
    });

    let server_state = state::new_state(registry, PipelineConfig::default());
    handlers::spawn_job_reaper(server_state.clone());

    let app = routes::router(server_state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{addr}")
}

#[tokio::test]
async fn test_health_endpoints() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Liveness
    let resp = client
        .get(format!("{base}/health/live"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Readiness
    let resp = client
        .get(format!("{base}/health/ready"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ready");
    assert_eq!(body["registered_jobs"], 2);
    assert_eq!(body["active_jobs"], 0);
}

#[tokio::test]
async fn test_submit_and_complete() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Submit fast-job
    let resp = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "fast-job"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["job_name"], "fast-job");
    assert_eq!(body["status"], "running");
    let job_id = body["job_id"].as_str().unwrap().to_string();

    // Wait for reaper to mark it completed.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let resp = client
        .get(format!("{base}/api/v1/jobs/{job_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "completed");
}

#[tokio::test]
async fn test_list_jobs() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Submit two jobs
    client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "long-job"}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "long-job"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .get(format!("{base}/api/v1/jobs"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["jobs"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_submit_unknown_job() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "nonexistent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_cancel_running_job() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Submit long-job
    let resp = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "long-job"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let job_id = body["job_id"].as_str().unwrap().to_string();

    // Cancel it
    let resp = client
        .post(format!("{base}/api/v1/jobs/{job_id}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "cancelling");

    // Wait for reaper
    tokio::time::sleep(Duration::from_secs(3)).await;

    let resp = client
        .get(format!("{base}/api/v1/jobs/{job_id}"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "cancelled");
}

#[tokio::test]
async fn test_cancel_completed_job() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // Submit fast-job and wait for it to complete
    let resp = client
        .post(format!("{base}/api/v1/jobs"))
        .json(&serde_json::json!({"job_name": "fast-job"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let job_id = body["job_id"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_secs(3)).await;

    // Try to cancel completed job — should be 409
    let resp = client
        .post(format!("{base}/api/v1/jobs/{job_id}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn test_get_unknown_job() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    let resp = client
        .get(format!("{base}/api/v1/jobs/{fake_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
