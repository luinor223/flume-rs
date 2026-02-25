//! Manages gRPC connections between TaskManagers for data exchange.
//!
//! Maintains one connection per remote TM address and provides
//! bidirectional streaming clients for data exchange.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tracing::{debug, warn};

use crate::proto::exchange::data_exchange_service_client::DataExchangeServiceClient;
use crate::proto::exchange::{CreditGrant, DataBatch};

/// A handle to an active exchange connection with a remote TaskManager.
pub struct ExchangeConnection {
    /// Send batches to the remote TM.
    pub batch_tx: mpsc::Sender<DataBatch>,
    /// Receive credit grants from the remote TM.
    pub credit_rx: mpsc::Receiver<CreditGrant>,
}

/// Manages gRPC connections to remote TaskManagers for data exchange.
///
/// Each connection maintains a bidirectional stream. Batches are sent
/// via `batch_tx` and credit grants are received via `credit_rx`.
pub struct ConnectionManager {
    /// Map from remote TM address to the gRPC channel.
    channels: HashMap<String, Channel>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    /// Get or create a gRPC channel to the given address.
    pub async fn get_or_connect(
        &mut self,
        address: &str,
    ) -> Result<Channel, tonic::transport::Error> {
        if let Some(channel) = self.channels.get(address) {
            return Ok(channel.clone());
        }

        let channel = Channel::from_shared(address.to_string())
            .expect("valid URI")
            .connect()
            .await?;

        self.channels.insert(address.to_string(), channel.clone());
        Ok(channel)
    }

    /// Open a bidirectional exchange stream with a remote TaskManager.
    ///
    /// Returns an `ExchangeConnection` with channels for sending batches
    /// and receiving credit grants.
    pub async fn open_exchange(
        &mut self,
        address: &str,
        outbound_buffer: usize,
    ) -> Result<ExchangeConnection, tonic::Status> {
        let channel = self
            .get_or_connect(address)
            .await
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;

        let mut client = DataExchangeServiceClient::new(channel);

        // Channel for the caller to send batches into.
        let (batch_tx, batch_rx) = mpsc::channel::<DataBatch>(outbound_buffer);

        // Channel for credit grants to be received by the caller.
        let (credit_tx, credit_rx) = mpsc::channel::<CreditGrant>(64);

        // Open the bidirectional stream.
        let response = client.exchange_data(ReceiverStream::new(batch_rx)).await?;

        let mut inbound = response.into_inner();

        // Spawn a task to forward credit grants from the stream to the channel.
        let addr = address.to_string();
        tokio::spawn(async move {
            while let Some(result) = inbound.next().await {
                match result {
                    Ok(grant) => {
                        if credit_tx.send(grant).await.is_err() {
                            debug!(address = %addr, "credit receiver dropped");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(address = %addr, error = %e, "exchange stream error");
                        break;
                    }
                }
            }
        });

        Ok(ExchangeConnection {
            batch_tx,
            credit_rx,
        })
    }

    /// Close a connection to a remote TM.
    pub fn disconnect(&mut self, address: &str) {
        self.channels.remove(address);
    }

    /// Number of active connections.
    pub fn connection_count(&self) -> usize {
        self.channels.len()
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_manager_new() {
        let mgr = ConnectionManager::new();
        assert_eq!(mgr.connection_count(), 0);
    }

    #[test]
    fn test_connection_manager_disconnect() {
        let mut mgr = ConnectionManager::new();
        // Disconnecting a non-existent address is a no-op.
        mgr.disconnect("http://127.0.0.1:50000");
        assert_eq!(mgr.connection_count(), 0);
    }
}
