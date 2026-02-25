//! TaskManager orchestration — combines gRPC server, JM client, heartbeat, and event loop.

use std::sync::Arc;
use std::time::Duration;

use flume_network::jm_client::JmClient;
use flume_network::proto::taskmanager::task_manager_service_server::TaskManagerServiceServer;
use flume_network::slot_manager::SlotManager;
use flume_network::tm_service::{TaskManagerServiceImpl, TmEvent};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::registry::JobRegistry;

/// Configuration for a TaskManager instance.
pub struct TaskManagerConfig {
    /// Unique identifier for this TM.
    pub id: String,
    /// Address to bind the gRPC server on.
    pub grpc_bind: String,
    /// Address of the JobManager to connect to.
    pub jm_address: String,
    /// Number of execution slots.
    pub num_slots: u32,
    /// Heartbeat interval.
    pub heartbeat_interval: Duration,
}

/// Run a TaskManager instance.
///
/// This starts:
/// 1. A gRPC server to receive deploy/cancel/barrier commands from the JM
/// 2. A heartbeat loop that periodically reports slot usage to the JM
/// 3. An event loop that processes TmEvents (deploy, cancel, barrier)
pub async fn run_task_manager(
    config: TaskManagerConfig,
    registry: Arc<JobRegistry>,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let slot_manager = Arc::new(Mutex::new(SlotManager::new(config.num_slots as usize)));

    // Start gRPC server.
    let (event_tx, event_rx) = mpsc::channel(32);
    let service = TaskManagerServiceImpl::new(event_tx);
    let listener = TcpListener::bind(&config.grpc_bind).await?;
    let local_addr = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let server_cancel = cancel.clone();
    tokio::spawn(async move {
        info!(address = %local_addr, "TM gRPC server starting");
        let result = Server::builder()
            .add_service(TaskManagerServiceServer::new(service))
            .serve_with_incoming_shutdown(incoming, server_cancel.cancelled())
            .await;
        if let Err(e) = result {
            error!(error = %e, "TM gRPC server error");
        }
    });

    // Give server time to start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Register with JobManager.
    let mut jm_client = JmClient::connect(&config.jm_address).await?;
    let job_names = registry.list_names();
    let accepted = jm_client
        .register(
            &config.id,
            &format!("http://{local_addr}"),
            config.num_slots,
            job_names,
        )
        .await?;

    if !accepted {
        return Err("JobManager rejected registration".into());
    }
    info!(tm_id = %config.id, "registered with JobManager");

    // Spawn heartbeat loop.
    let heartbeat_cancel = cancel.clone();
    let heartbeat_sm = slot_manager.clone();
    let heartbeat_id = config.id.clone();
    let heartbeat_interval = config.heartbeat_interval;
    let mut heartbeat_client = JmClient::connect(&config.jm_address).await?;

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval);
        loop {
            tokio::select! {
                _ = heartbeat_cancel.cancelled() => break,
                _ = interval.tick() => {
                    let sm = heartbeat_sm.lock().await;
                    let available = sm.slots_available() as u32;
                    let in_use = sm.slots_in_use() as u32;
                    drop(sm);

                    match heartbeat_client.heartbeat(&heartbeat_id, available, in_use).await {
                        Ok(true) => {}
                        Ok(false) => {
                            warn!(tm_id = %heartbeat_id, "JM told us to stop, cancelling");
                            heartbeat_cancel.cancel();
                            break;
                        }
                        Err(e) => {
                            warn!(error = %e, "heartbeat failed");
                        }
                    }
                }
            }
        }
    });

    // Event loop.
    run_event_loop(event_rx, slot_manager, &config.id, cancel).await;

    Ok(())
}

/// Process TmEvents from the gRPC service.
async fn run_event_loop(
    mut event_rx: mpsc::Receiver<TmEvent>,
    slot_manager: Arc<Mutex<SlotManager>>,
    tm_id: &str,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!(tm_id = %tm_id, "TM event loop shutting down");
                break;
            }
            event = event_rx.recv() => {
                match event {
                    Some(TmEvent::DeploySubtask { job_id, subtask_id, reply, .. }) => {
                        let mut sm = slot_manager.lock().await;
                        match sm.deploy(&job_id, &subtask_id) {
                            Some(slot_idx) => {
                                info!(
                                    job_id = %job_id,
                                    subtask_id = %subtask_id,
                                    slot = slot_idx,
                                    "deployed subtask"
                                );
                                let _ = reply.send(Ok(()));
                            }
                            None => {
                                warn!(job_id = %job_id, subtask_id = %subtask_id, "no free slots");
                                let _ = reply.send(Err("no free slots".into()));
                            }
                        }
                    }
                    Some(TmEvent::CancelSubtask { subtask_id, reply, .. }) => {
                        let mut sm = slot_manager.lock().await;
                        let cancelled = sm.cancel(&subtask_id);
                        if cancelled {
                            info!(subtask_id = %subtask_id, "cancelled subtask");
                        }
                        let _ = reply.send(cancelled);
                    }
                    Some(TmEvent::InjectBarrier { job_id, checkpoint_id }) => {
                        info!(
                            job_id = %job_id,
                            checkpoint_id = checkpoint_id,
                            "received checkpoint barrier"
                        );
                    }
                    None => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tm_config() {
        let config = TaskManagerConfig {
            id: "tm-1".into(),
            grpc_bind: "127.0.0.1:0".into(),
            jm_address: "http://127.0.0.1:50051".into(),
            num_slots: 4,
            heartbeat_interval: Duration::from_secs(5),
        };
        assert_eq!(config.id, "tm-1");
        assert_eq!(config.num_slots, 4);
    }
}
