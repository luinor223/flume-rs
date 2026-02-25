//! Distributed networking layer for flume-rs.
//!
//! Provides gRPC service definitions and data exchange protocols
//! for cross-TaskManager communication.

pub mod connection_manager;
pub mod exchange;
pub mod jm_client;
pub mod jm_service;
pub mod network_collector;
pub mod network_input;
pub mod resource_manager;
pub mod tm_client;
pub mod tm_service;

pub mod proto {
    pub mod jobmanager {
        tonic::include_proto!("flume.jobmanager");
    }
    pub mod taskmanager {
        tonic::include_proto!("flume.taskmanager");
    }
    pub mod exchange {
        tonic::include_proto!("flume.exchange");
    }
}

#[cfg(test)]
mod tests {
    use super::proto::*;

    #[test]
    fn test_proto_register_request() {
        let req = jobmanager::RegisterRequest {
            task_manager_id: "tm-1".into(),
            address: "127.0.0.1:50052".into(),
            num_slots: 4,
            registered_jobs: vec!["job-a".into(), "job-b".into()],
        };
        assert_eq!(req.task_manager_id, "tm-1");
        assert_eq!(req.num_slots, 4);
        assert_eq!(req.registered_jobs.len(), 2);
    }

    #[test]
    fn test_proto_deploy_request() {
        let req = taskmanager::DeployRequest {
            job_id: "job-123".into(),
            job_name: "my-job".into(),
            pipeline_config: vec![1, 2, 3],
            physical_graph: vec![4, 5, 6],
            subtask_id: "map-0".into(),
        };
        assert_eq!(req.job_name, "my-job");
        assert_eq!(req.subtask_id, "map-0");
    }

    #[test]
    fn test_proto_data_batch() {
        let batch = exchange::DataBatch {
            channel_id: "src-0->map-0".into(),
            payload: vec![10, 20, 30],
            record_count: 3,
            is_barrier: false,
            checkpoint_id: 0,
        };
        assert_eq!(batch.record_count, 3);
        assert!(!batch.is_barrier);
    }

    #[test]
    fn test_proto_prost_roundtrip() {
        use prost::Message;

        let req = jobmanager::RegisterRequest {
            task_manager_id: "tm-1".into(),
            address: "127.0.0.1:50052".into(),
            num_slots: 4,
            registered_jobs: vec!["job-a".into()],
        };
        let bytes = req.encode_to_vec();
        let decoded = jobmanager::RegisterRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.task_manager_id, "tm-1");
        assert_eq!(decoded.num_slots, 4);
    }
}
