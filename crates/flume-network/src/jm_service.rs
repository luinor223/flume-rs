//! gRPC server implementation for the JobManager service.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::proto::jobmanager::job_manager_service_server::JobManagerService;
use crate::proto::jobmanager::*;
use crate::resource_manager::ResourceManager;

/// Events sent by the gRPC service to the JM main loop.
#[derive(Debug)]
pub enum JmEvent {
    TaskManagerRegistered {
        id: String,
        address: String,
        num_slots: u32,
    },
    HeartbeatReceived {
        id: String,
    },
    CheckpointAcknowledged {
        job_id: String,
        subtask_id: String,
        checkpoint_id: u64,
        state: Vec<u8>,
    },
    TaskStatusReport {
        tm_id: String,
        subtask_id: String,
        status: String,
        error: String,
    },
}

/// Implementation of the JobManager gRPC service.
pub struct JobManagerServiceImpl {
    resource_manager: Arc<Mutex<ResourceManager>>,
    event_tx: mpsc::Sender<JmEvent>,
}

impl JobManagerServiceImpl {
    pub fn new(
        resource_manager: Arc<Mutex<ResourceManager>>,
        event_tx: mpsc::Sender<JmEvent>,
    ) -> Self {
        Self {
            resource_manager,
            event_tx,
        }
    }
}

#[tonic::async_trait]
impl JobManagerService for JobManagerServiceImpl {
    async fn register_task_manager(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();
        info!(
            tm_id = %req.task_manager_id,
            addr = %req.address,
            slots = req.num_slots,
            "TaskManager registering"
        );

        let accepted = self.resource_manager.lock().await.register(&req);

        let _ = self
            .event_tx
            .send(JmEvent::TaskManagerRegistered {
                id: req.task_manager_id.clone(),
                address: req.address.clone(),
                num_slots: req.num_slots,
            })
            .await;

        Ok(Response::new(RegisterResponse {
            accepted,
            message: if accepted {
                "registered".into()
            } else {
                "rejected".into()
            },
        }))
    }

    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();
        let known = self.resource_manager.lock().await.heartbeat(&req);

        if known {
            let _ = self
                .event_tx
                .send(JmEvent::HeartbeatReceived {
                    id: req.task_manager_id.clone(),
                })
                .await;
        }

        Ok(Response::new(HeartbeatResponse {
            should_continue: known,
        }))
    }

    async fn acknowledge_checkpoint(
        &self,
        request: Request<CheckpointAck>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let _ = self
            .event_tx
            .send(JmEvent::CheckpointAcknowledged {
                job_id: req.job_id,
                subtask_id: req.subtask_id,
                checkpoint_id: req.checkpoint_id,
                state: req.state_bytes,
            })
            .await;
        Ok(Response::new(Empty {}))
    }

    async fn report_task_status(
        &self,
        request: Request<TaskStatusReport>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let _ = self
            .event_tx
            .send(JmEvent::TaskStatusReport {
                tm_id: req.task_manager_id,
                subtask_id: req.subtask_id,
                status: req.status,
                error: req.error_message,
            })
            .await;
        Ok(Response::new(Empty {}))
    }
}
