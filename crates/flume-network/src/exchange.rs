//! gRPC implementation of the DataExchangeService.
//!
//! Implements credit-based flow control: the receiver grants credits
//! for buffer capacity, and the sender only sends batches up to
//! available credits.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::debug;

use crate::proto::exchange::data_exchange_service_server::DataExchangeService;
use crate::proto::exchange::{CreditGrant, DataBatch};

/// Received data batches are forwarded to channel subscribers keyed by channel_id.
pub type BatchSender = mpsc::Sender<DataBatch>;
pub type BatchReceiver = mpsc::Receiver<DataBatch>;

/// Manages incoming data channels for a TaskManager.
///
/// When a remote TM opens a bidirectional stream, received batches
/// are routed to the appropriate local channel subscriber.
#[derive(Clone)]
pub struct ExchangeRouter {
    /// Map from channel_id to the local receiver's sender handle.
    subscribers: Arc<Mutex<HashMap<String, BatchSender>>>,
    /// Initial credits granted to each new channel.
    pub initial_credits: u32,
}

impl ExchangeRouter {
    pub fn new(initial_credits: u32) -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(HashMap::new())),
            initial_credits,
        }
    }

    /// Register a local channel to receive batches for the given channel_id.
    pub async fn subscribe(&self, channel_id: &str, sender: BatchSender) {
        self.subscribers
            .lock()
            .await
            .insert(channel_id.to_string(), sender);
    }

    /// Remove a channel subscription.
    pub async fn unsubscribe(&self, channel_id: &str) {
        self.subscribers.lock().await.remove(channel_id);
    }

    /// Route a batch to the appropriate local subscriber.
    /// Returns the channel_id if a subscriber exists.
    async fn route(&self, batch: &DataBatch) -> Option<String> {
        let subs = self.subscribers.lock().await;
        if let Some(tx) = subs.get(&batch.channel_id)
            && tx.send(batch.clone()).await.is_ok()
        {
            return Some(batch.channel_id.clone());
        }
        None
    }
}

/// Implementation of the DataExchangeService gRPC service.
pub struct DataExchangeServiceImpl {
    router: ExchangeRouter,
}

impl DataExchangeServiceImpl {
    pub fn new(router: ExchangeRouter) -> Self {
        Self { router }
    }
}

#[tonic::async_trait]
impl DataExchangeService for DataExchangeServiceImpl {
    type ExchangeDataStream = ReceiverStream<Result<CreditGrant, Status>>;

    async fn exchange_data(
        &self,
        request: Request<Streaming<DataBatch>>,
    ) -> Result<Response<Self::ExchangeDataStream>, Status> {
        let mut inbound = request.into_inner();
        let router = self.router.clone();
        let initial_credits = router.initial_credits;

        // Channel for sending credit grants back to the sender.
        let (credit_tx, credit_rx) = mpsc::channel(64);

        tokio::spawn(async move {
            // Track which channels we've seen to send initial credits.
            let mut seen_channels: HashMap<String, bool> = HashMap::new();

            while let Some(result) = inbound.next().await {
                match result {
                    Ok(batch) => {
                        let channel_id = batch.channel_id.clone();

                        // Send initial credits for new channels.
                        if !seen_channels.contains_key(&channel_id) {
                            seen_channels.insert(channel_id.clone(), true);
                            let _ = credit_tx
                                .send(Ok(CreditGrant {
                                    channel_id: channel_id.clone(),
                                    credits: initial_credits,
                                }))
                                .await;
                        }

                        // Route the batch to the local subscriber.
                        if router.route(&batch).await.is_some() {
                            // Grant a credit back for each successfully delivered batch.
                            let _ = credit_tx
                                .send(Ok(CreditGrant {
                                    channel_id,
                                    credits: 1,
                                }))
                                .await;
                        } else {
                            debug!(
                                channel_id = %channel_id,
                                "no subscriber for channel, dropping batch"
                            );
                        }
                    }
                    Err(e) => {
                        debug!(error = %e, "exchange stream error");
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(credit_rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exchange_router_subscribe_and_route() {
        let router = ExchangeRouter::new(16);
        let (tx, mut rx) = mpsc::channel(8);

        router.subscribe("ch-1", tx).await;

        let batch = DataBatch {
            channel_id: "ch-1".into(),
            payload: vec![1, 2, 3],
            record_count: 1,
            is_barrier: false,
            checkpoint_id: 0,
        };

        let result = router.route(&batch).await;
        assert_eq!(result, Some("ch-1".to_string()));

        let received = rx.recv().await.unwrap();
        assert_eq!(received.payload, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_exchange_router_no_subscriber() {
        let router = ExchangeRouter::new(16);

        let batch = DataBatch {
            channel_id: "ch-missing".into(),
            payload: vec![],
            record_count: 0,
            is_barrier: false,
            checkpoint_id: 0,
        };

        let result = router.route(&batch).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_exchange_router_unsubscribe() {
        let router = ExchangeRouter::new(16);
        let (tx, _rx) = mpsc::channel(8);

        router.subscribe("ch-1", tx).await;
        router.unsubscribe("ch-1").await;

        let batch = DataBatch {
            channel_id: "ch-1".into(),
            payload: vec![],
            record_count: 0,
            is_barrier: false,
            checkpoint_id: 0,
        };

        let result = router.route(&batch).await;
        assert!(result.is_none());
    }
}
