//! gRPC server implementation for the TaskManager service.

use tokio::sync::{mpsc, oneshot};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::proto::taskmanager::task_manager_service_server::TaskManagerService;
use crate::proto::taskmanager::*;

/// Events sent by the gRPC service to the TM main loop.
#[derive(Debug)]
pub enum TmEvent {
    DeploySubtask {
        job_id: String,
        job_name: String,
        pipeline_config_bytes: Vec<u8>,
        physical_graph_bytes: Vec<u8>,
        subtask_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    CancelSubtask {
        job_id: String,
        subtask_id: String,
        reply: oneshot::Sender<bool>,
    },
    InjectBarrier {
        job_id: String,
        checkpoint_id: u64,
    },
}

/// Implementation of the TaskManager gRPC service.
pub struct TaskManagerServiceImpl {
    event_tx: mpsc::Sender<TmEvent>,
}

impl TaskManagerServiceImpl {
    pub fn new(event_tx: mpsc::Sender<TmEvent>) -> Self {
        Self { event_tx }
    }
}

#[tonic::async_trait]
impl TaskManagerService for TaskManagerServiceImpl {
    async fn deploy_subtask(
        &self,
        request: Request<DeployRequest>,
    ) -> Result<Response<DeployResponse>, Status> {
        let req = request.into_inner();
        info!(
            job_id = %req.job_id,
            subtask_id = %req.subtask_id,
            "deploy subtask request received"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.event_tx
            .send(TmEvent::DeploySubtask {
                job_id: req.job_id,
                job_name: req.job_name,
                pipeline_config_bytes: req.pipeline_config,
                physical_graph_bytes: req.physical_graph,
                subtask_id: req.subtask_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Status::internal("TM event loop closed"))?;

        match reply_rx.await {
            Ok(Ok(())) => Ok(Response::new(DeployResponse {
                accepted: true,
                message: "deployed".into(),
            })),
            Ok(Err(msg)) => Ok(Response::new(DeployResponse {
                accepted: false,
                message: msg,
            })),
            Err(_) => Err(Status::internal("deploy reply dropped")),
        }
    }

    async fn cancel_subtask(
        &self,
        request: Request<CancelRequest>,
    ) -> Result<Response<CancelResponse>, Status> {
        let req = request.into_inner();
        info!(
            job_id = %req.job_id,
            subtask_id = %req.subtask_id,
            "cancel subtask request received"
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.event_tx
            .send(TmEvent::CancelSubtask {
                job_id: req.job_id,
                subtask_id: req.subtask_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Status::internal("TM event loop closed"))?;

        match reply_rx.await {
            Ok(cancelled) => Ok(Response::new(CancelResponse { cancelled })),
            Err(_) => Err(Status::internal("cancel reply dropped")),
        }
    }

    async fn inject_checkpoint_barrier(
        &self,
        request: Request<BarrierRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        self.event_tx
            .send(TmEvent::InjectBarrier {
                job_id: req.job_id,
                checkpoint_id: req.checkpoint_id,
            })
            .await
            .map_err(|_| Status::internal("TM event loop closed"))?;
        Ok(Response::new(Empty {}))
    }
}
