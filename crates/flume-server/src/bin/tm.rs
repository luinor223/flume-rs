use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use flume_api::{PrintSink, StreamExecutionEnvironment, init_logging};
use flume_server::registry::JobRegistry;
use flume_server::tm::{TaskManagerConfig, run_task_manager};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Parser)]
#[command(name = "flume-tm", about = "Flume TaskManager process")]
struct Args {
    /// Unique TaskManager ID.
    #[arg(long, env = "FLUME_TM_ID", default_value = "tm-1")]
    id: String,

    /// Address to bind the gRPC server on.
    #[arg(long, env = "FLUME_TM_BIND", default_value = "127.0.0.1:0")]
    bind: String,

    /// JobManager gRPC address to connect to.
    #[arg(
        long,
        env = "FLUME_JM_ADDRESS",
        default_value = "http://127.0.0.1:50051"
    )]
    jm_address: String,

    /// Number of execution slots.
    #[arg(long, env = "FLUME_TM_SLOTS", default_value = "4")]
    slots: u32,

    /// Heartbeat interval in seconds.
    #[arg(long, default_value = "5")]
    heartbeat_interval: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_logging(false);
    let args = Args::parse();

    info!(
        tm_id = %args.id,
        bind = %args.bind,
        jm = %args.jm_address,
        slots = args.slots,
        "starting TaskManager"
    );

    let mut registry = JobRegistry::new();
    register_example_jobs(&mut registry);

    let config = TaskManagerConfig {
        id: args.id,
        grpc_bind: args.bind,
        jm_address: args.jm_address,
        num_slots: args.slots,
        heartbeat_interval: Duration::from_secs(args.heartbeat_interval),
    };

    let cancel = CancellationToken::new();

    // Handle shutdown signals.
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal_cancel.cancel();
    });

    run_task_manager(config, Arc::new(registry), cancel).await?;

    info!("TaskManager shut down");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => { info!("received SIGINT"); }
            _ = sigterm.recv() => { info!("received SIGTERM"); }
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("received Ctrl+C");
    }
}

fn register_example_jobs(registry: &mut JobRegistry) {
    registry.register("example-doubler", |config, cancel| async move {
        let mut env = StreamExecutionEnvironment::from_config(config);
        env.connect_cancel_token(cancel);
        env.from_collection(vec![1, 2, 3, 4, 5])
            .map(|x| x * 2)
            .add_sink(PrintSink::new("doubled"))
            .await
    });
}
