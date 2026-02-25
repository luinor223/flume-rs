use std::net::SocketAddr;

use clap::{Parser, ValueEnum};
use flume_api::{PrintSink, StreamExecutionEnvironment, init_logging};
use flume_server::api::{handlers, routes, state};
use flume_server::config::load_server_config;
use flume_server::registry::JobRegistry;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Server operating mode.
#[derive(Debug, Clone, ValueEnum)]
enum ServerMode {
    /// Standalone mode — no distributed networking.
    Standalone,
    /// JobManager mode — accepts TM registrations and distributes work.
    Jm,
}

#[derive(Parser)]
#[command(name = "flume-server", about = "Flume streaming engine server")]
struct Args {
    /// Path to server TOML config file.
    #[arg(long)]
    config: Option<String>,

    /// Override bind address for HTTP (e.g. 0.0.0.0:8080).
    #[arg(long)]
    bind: Option<SocketAddr>,

    /// Server mode: standalone or jm (JobManager).
    #[arg(long, value_enum, default_value = "standalone")]
    mode: ServerMode,

    /// gRPC port for JM mode (default: 50051).
    #[arg(long, default_value = "50051")]
    grpc_port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_logging(false);
    let args = Args::parse();

    let mut server_config = load_server_config(args.config.as_deref().map(std::path::Path::new))?;
    if let Some(bind) = args.bind {
        server_config.bind_addr = bind;
    }

    info!(bind = %server_config.bind_addr, mode = ?args.mode, "starting flume-server");

    let mut registry = JobRegistry::new();
    register_example_jobs(&mut registry);
    info!(
        registered_jobs = registry.len(),
        names = ?registry.list_names(),
        "job registry initialized"
    );

    let server_state = state::new_state(registry, server_config.pipeline);
    handlers::spawn_job_reaper(server_state.clone());

    let cancel = CancellationToken::new();

    // Start JM gRPC server in JM mode.
    if matches!(args.mode, ServerMode::Jm) {
        let grpc_bind = format!("0.0.0.0:{}", args.grpc_port);
        flume_server::jm::run_jm_grpc(&grpc_bind, server_state.clone(), cancel.clone()).await?;
        info!(grpc_port = args.grpc_port, "JM gRPC server started");
    }

    let app = routes::router(server_state);
    let listener = TcpListener::bind(server_config.bind_addr).await?;
    info!(addr = %server_config.bind_addr, "server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel))
        .await?;

    info!("server shut down");
    Ok(())
}

async fn shutdown_signal(cancel: CancellationToken) {
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
    cancel.cancel();
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
