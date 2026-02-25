//! Client wrapper for JobManager-to-TaskManager gRPC calls.

use tonic::transport::Channel;

use crate::proto::taskmanager::task_manager_service_client::TaskManagerServiceClient;
use crate::proto::taskmanager::*;

/// Client for calling the TaskManager gRPC service.
pub struct TmClient {
    client: TaskManagerServiceClient<Channel>,
}

impl TmClient {
    pub async fn connect(addr: &str) -> Result<Self, tonic::transport::Error> {
        let client = TaskManagerServiceClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }

    pub fn from_channel(channel: Channel) -> Self {
        Self {
            client: TaskManagerServiceClient::new(channel),
        }
    }

    /// Deploy a subtask on the remote TaskManager.
    pub async fn deploy_subtask(
        &mut self,
        job_id: &str,
        job_name: &str,
        config: &[u8],
        graph: &[u8],
        subtask_id: &str,
    ) -> Result<bool, tonic::Status> {
        let resp = self
            .client
            .deploy_subtask(DeployRequest {
                job_id: job_id.into(),
                job_name: job_name.into(),
                pipeline_config: config.to_vec(),
                physical_graph: graph.to_vec(),
                subtask_id: subtask_id.into(),
            })
            .await?;
        Ok(resp.into_inner().accepted)
    }

    /// Cancel a subtask on the remote TaskManager.
    pub async fn cancel_subtask(
        &mut self,
        job_id: &str,
        subtask_id: &str,
    ) -> Result<bool, tonic::Status> {
        let resp = self
            .client
            .cancel_subtask(CancelRequest {
                job_id: job_id.into(),
                subtask_id: subtask_id.into(),
            })
            .await?;
        Ok(resp.into_inner().cancelled)
    }

    /// Inject a checkpoint barrier on the remote TaskManager.
    pub async fn inject_barrier(
        &mut self,
        job_id: &str,
        checkpoint_id: u64,
    ) -> Result<(), tonic::Status> {
        self.client
            .inject_checkpoint_barrier(BarrierRequest {
                job_id: job_id.into(),
                checkpoint_id,
            })
            .await?;
        Ok(())
    }
}
