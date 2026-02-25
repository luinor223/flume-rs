use clap::Parser;
use flume_server::cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = flume_server::cli::run(cli).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
