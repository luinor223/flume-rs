//! Integration tests for JM and TM gRPC services.

use std::sync::Arc;
use std::time::Duration;

use flume_network::jm_client::JmClient;
use flume_network::jm_service::*;
use flume_network::proto::jobmanager::job_manager_service_server::JobManagerServiceServer;
use flume_network::proto::taskmanager::task_manager_service_server::TaskManagerServiceServer;
use flume_network::resource_manager::ResourceManager;
use flume_network::tm_client::TmClient;
use flume_network::tm_service::*;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tonic::transport::Server;

/// Start a JM gRPC server on a random port, return the address and event receiver.
async fn start_jm_server() -> (String, mpsc::Receiver<JmEvent>) {
    let rm = Arc::new(Mutex::new(ResourceManager::new(Duration::from_secs(30))));
    let (event_tx, event_rx) = mpsc::channel(32);
    let service = JobManagerServiceImpl::new(rm, event_tx);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(JobManagerServiceServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    // Small delay for server startup.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (format!("http://{addr}"), event_rx)
}

/// Start a TM gRPC server on a random port, return the address and event receiver.
async fn start_tm_server() -> (String, mpsc::Receiver<TmEvent>) {
    let (event_tx, event_rx) = mpsc::channel(32);
    let service = TaskManagerServiceImpl::new(event_tx);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(TaskManagerServiceServer::new(service))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (format!("http://{addr}"), event_rx)
}

#[tokio::test]
async fn test_register_and_heartbeat() {
    let (addr, mut events) = start_jm_server().await;
    let mut client = JmClient::connect(&addr).await.unwrap();

    // Register.
    let accepted = client
        .register("tm-1", "127.0.0.1:50052", 4, vec!["job-a".into()])
        .await
        .unwrap();
    assert!(accepted);

    // Check event.
    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        JmEvent::TaskManagerRegistered { id, num_slots, .. } if id == "tm-1" && num_slots == 4
    ));

    // Heartbeat.
    let should_continue = client.heartbeat("tm-1", 3, 1).await.unwrap();
    assert!(should_continue);

    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        JmEvent::HeartbeatReceived { id } if id == "tm-1"
    ));
}

#[tokio::test]
async fn test_heartbeat_unknown_tm() {
    let (addr, _events) = start_jm_server().await;
    let mut client = JmClient::connect(&addr).await.unwrap();

    // Heartbeat for unregistered TM.
    let should_continue = client.heartbeat("tm-unknown", 0, 0).await.unwrap();
    assert!(!should_continue);
}

#[tokio::test]
async fn test_deploy_subtask() {
    let (addr, mut events) = start_tm_server().await;
    let mut client = TmClient::connect(&addr).await.unwrap();

    // Spawn a handler that accepts the deploy.
    tokio::spawn(async move {
        if let Some(TmEvent::DeploySubtask { reply, .. }) = events.recv().await {
            reply.send(Ok(())).ok();
        }
    });

    let accepted = client
        .deploy_subtask("job-1", "my-job", b"config", b"graph", "map-0")
        .await
        .unwrap();
    assert!(accepted);
}

#[tokio::test]
async fn test_cancel_subtask() {
    let (addr, mut events) = start_tm_server().await;
    let mut client = TmClient::connect(&addr).await.unwrap();

    tokio::spawn(async move {
        if let Some(TmEvent::CancelSubtask { reply, .. }) = events.recv().await {
            reply.send(true).ok();
        }
    });

    let cancelled = client.cancel_subtask("job-1", "map-0").await.unwrap();
    assert!(cancelled);
}

#[tokio::test]
async fn test_inject_barrier() {
    let (addr, mut events) = start_tm_server().await;
    let mut client = TmClient::connect(&addr).await.unwrap();

    client.inject_barrier("job-1", 42).await.unwrap();

    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        TmEvent::InjectBarrier { job_id, checkpoint_id } if job_id == "job-1" && checkpoint_id == 42
    ));
}

#[tokio::test]
async fn test_report_task_status() {
    let (addr, mut events) = start_jm_server().await;
    let mut client = JmClient::connect(&addr).await.unwrap();

    client
        .report_task_status("tm-1", "map-0", "completed", "")
        .await
        .unwrap();

    let event = events.recv().await.unwrap();
    assert!(matches!(
        event,
        JmEvent::TaskStatusReport { subtask_id, status, .. } if subtask_id == "map-0" && status == "completed"
    ));
}
