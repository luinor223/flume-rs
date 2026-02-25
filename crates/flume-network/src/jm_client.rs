//! Client wrapper for TaskManager-to-JobManager gRPC calls.

use tonic::transport::Channel;

use crate::proto::jobmanager::job_manager_service_client::JobManagerServiceClient;
use crate::proto::jobmanager::*;

/// Client for calling the JobManager gRPC service.
pub struct JmClient {
    client: JobManagerServiceClient<Channel>,
}

impl JmClient {
    pub async fn connect(addr: &str) -> Result<Self, tonic::transport::Error> {
        let client = JobManagerServiceClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }

    pub fn from_channel(channel: Channel) -> Self {
        Self {
            client: JobManagerServiceClient::new(channel),
        }
    }

    /// Register this TaskManager with the JobManager.
    pub async fn register(
        &mut self,
        tm_id: &str,
        address: &str,
        num_slots: u32,
        job_names: Vec<String>,
    ) -> Result<bool, tonic::Status> {
        let resp = self
            .client
            .register_task_manager(RegisterRequest {
                task_manager_id: tm_id.into(),
                address: address.into(),
                num_slots,
                registered_jobs: job_names,
            })
            .await?;
        Ok(resp.into_inner().accepted)
    }

    /// Send a heartbeat to the JobManager.
    pub async fn heartbeat(
        &mut self,
        tm_id: &str,
        available: u32,
        in_use: u32,
    ) -> Result<bool, tonic::Status> {
        let resp = self
            .client
            .heartbeat(HeartbeatRequest {
                task_manager_id: tm_id.into(),
                slots_available: available,
                slots_in_use: in_use,
            })
            .await?;
        Ok(resp.into_inner().should_continue)
    }

    /// Report a subtask's status to the JobManager.
    pub async fn report_task_status(
        &mut self,
        tm_id: &str,
        subtask_id: &str,
        status: &str,
        error: &str,
    ) -> Result<(), tonic::Status> {
        self.client
            .report_task_status(TaskStatusReport {
                task_manager_id: tm_id.into(),
                subtask_id: subtask_id.into(),
                status: status.into(),
                error_message: error.into(),
            })
            .await?;
        Ok(())
    }
}
