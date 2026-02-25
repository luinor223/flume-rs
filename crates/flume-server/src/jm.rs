//! JobManager orchestration — distributed job deployment to TaskManagers.

use std::sync::Arc;
use std::time::Duration;

use flume_network::jm_service::{JmEvent, JobManagerServiceImpl};
use flume_network::proto::jobmanager::job_manager_service_server::JobManagerServiceServer;
use flume_network::resource_manager::ResourceManager;
use flume_network::tm_client::TmClient;
use flume_runtime::physical::{SchedulingStrategy, schedule};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::api::state::ServerState;

/// Start the JM gRPC server and event processing loop.
pub async fn run_jm_grpc(
    bind_addr: &str,
    server_state: ServerState,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rm = Arc::new(Mutex::new(ResourceManager::new(Duration::from_secs(30))));

    // Store the resource manager in server state.
    {
        let mut inner = server_state.lock().await;
        inner.resource_manager = Some(rm.clone());
    }

    let (event_tx, event_rx) = mpsc::channel(64);
    let service = JobManagerServiceImpl::new(rm.clone(), event_tx);

    let listener = TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let server_cancel = cancel.clone();
    tokio::spawn(async move {
        info!(address = %local_addr, "JM gRPC server starting");
        let result = Server::builder()
            .add_service(JobManagerServiceServer::new(service))
            .serve_with_incoming_shutdown(incoming, server_cancel.cancelled())
            .await;
        if let Err(e) = result {
            error!(error = %e, "JM gRPC server error");
        }
    });

    // Process JM events.
    tokio::spawn(process_jm_events(event_rx, rm, server_state, cancel));

    Ok(())
}

/// Process events from the JM gRPC service.
async fn process_jm_events(
    mut event_rx: mpsc::Receiver<JmEvent>,
    rm: Arc<Mutex<ResourceManager>>,
    _server_state: ServerState,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("JM event loop shutting down");
                break;
            }
            event = event_rx.recv() => {
                match event {
                    Some(JmEvent::TaskManagerRegistered { id, address, num_slots }) => {
                        info!(tm_id = %id, address = %address, slots = num_slots, "TM registered");
                    }
                    Some(JmEvent::HeartbeatReceived { id }) => {
                        tracing::trace!(tm_id = %id, "heartbeat received");
                    }
                    Some(JmEvent::CheckpointAcknowledged { job_id, subtask_id, checkpoint_id, .. }) => {
                        info!(job_id = %job_id, subtask_id = %subtask_id, checkpoint_id = checkpoint_id, "checkpoint ack");
                    }
                    Some(JmEvent::TaskStatusReport { tm_id, subtask_id, status, error }) => {
                        if error.is_empty() {
                            info!(tm_id = %tm_id, subtask_id = %subtask_id, status = %status, "task status");
                        } else {
                            warn!(tm_id = %tm_id, subtask_id = %subtask_id, status = %status, error = %error, "task status");
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Dead TM detection loop is handled separately.
    let _ = rm;
}

/// Deploy a job across registered TaskManagers.
///
/// 1. Build a logical graph from the job name
/// 2. Get slot offers from the resource manager
/// 3. Schedule subtasks onto TM slots
/// 4. Send DeploySubtask RPCs to each TM
pub async fn deploy_job(
    server_state: &ServerState,
    job_id: &str,
    job_name: &str,
    parallelism: usize,
) -> Result<usize, String> {
    let inner = server_state.lock().await;

    let rm = inner
        .resource_manager
        .as_ref()
        .ok_or("no resource manager (JM gRPC not running)")?;

    let rm_guard = rm.lock().await;
    let slot_offers = rm_guard.slot_offers();

    if slot_offers.is_empty() {
        return Err("no TaskManagers registered".into());
    }

    let total_available: usize = slot_offers
        .iter()
        .map(|o| o.total_slots - o.used_slots)
        .sum();
    if total_available < parallelism {
        return Err(format!(
            "not enough slots: need {parallelism}, available {total_available}"
        ));
    }

    // Build a simple logical graph for the job.
    let graph = build_job_graph(job_name, parallelism);

    // Schedule subtasks.
    let physical = schedule(&graph, &slot_offers, SchedulingStrategy::Spread)
        .map_err(|e| format!("scheduling failed: {e}"))?;

    // Group assignments by TM address.
    let mut tm_assignments: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for assignment in &physical.assignments {
        let tm_id = &assignment.task_manager_id;
        if let Some(info) = rm_guard.get(tm_id) {
            tm_assignments
                .entry(info.address.clone())
                .or_default()
                .push(format!(
                    "{}-{}",
                    assignment.subtask_id.operator_name, assignment.subtask_id.subtask_index
                ));
        }
    }

    drop(rm_guard);
    drop(inner);

    // Send deploy RPCs.
    let mut deployed = 0;
    for (address, subtask_ids) in &tm_assignments {
        match TmClient::connect(address).await {
            Ok(mut client) => {
                for subtask_id in subtask_ids {
                    match client
                        .deploy_subtask(job_id, job_name, &[], &[], subtask_id)
                        .await
                    {
                        Ok(true) => {
                            deployed += 1;
                            info!(
                                job_id = %job_id,
                                subtask_id = %subtask_id,
                                address = %address,
                                "subtask deployed"
                            );
                        }
                        Ok(false) => {
                            warn!(subtask_id = %subtask_id, "deployment rejected by TM");
                        }
                        Err(e) => {
                            warn!(subtask_id = %subtask_id, error = %e, "deploy RPC failed");
                        }
                    }
                }
            }
            Err(e) => {
                warn!(address = %address, error = %e, "failed to connect to TM");
            }
        }
    }

    Ok(deployed)
}

/// Build a simple logical graph for a job.
fn build_job_graph(job_name: &str, parallelism: usize) -> flume_runtime::dag::LogicalGraph {
    use flume_runtime::dag::{LogicalGraph, NodeKind};

    let mut graph = LogicalGraph::new();
    graph.add_node(format!("{job_name}-source"), NodeKind::Source, parallelism);
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_job_graph() {
        let graph = build_job_graph("my-job", 4);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].parallelism, 4);
    }
}
