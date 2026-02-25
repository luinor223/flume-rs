//! CLI client for the flume-server REST API.

#[cfg(feature = "cli")]
use reqwest::Client;

/// Top-level CLI arguments.
#[derive(Debug, clap::Parser)]
#[command(name = "flume-cli", about = "CLI client for flume-server")]
pub struct Cli {
    /// Server endpoint URL.
    #[arg(long, default_value = "http://127.0.0.1:8080", env = "FLUME_ENDPOINT")]
    pub endpoint: String,

    /// Output format.
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub command: Command,
}

/// Output format for CLI responses.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Available CLI subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Submit a new job by name.
    Submit {
        /// Registered job name.
        job_name: String,
        /// Override parallelism.
        #[arg(long)]
        parallelism: Option<usize>,
    },
    /// Get the status of a job by ID.
    Status {
        /// Job UUID.
        job_id: String,
    },
    /// List all jobs.
    List,
    /// Cancel a running job by ID.
    Cancel {
        /// Job UUID.
        job_id: String,
    },
    /// Check server health.
    Health,
}

/// Run the CLI with the given arguments.
#[cfg(feature = "cli")]
pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let base = cli.endpoint.trim_end_matches('/');

    match cli.command {
        Command::Submit {
            job_name,
            parallelism,
        } => {
            let mut body = serde_json::json!({ "job_name": job_name });
            if let Some(p) = parallelism {
                body["parallelism"] = serde_json::json!(p);
            }
            let resp = client
                .post(format!("{base}/api/v1/jobs"))
                .json(&body)
                .send()
                .await?;
            print_response(resp, cli.format).await?;
        }
        Command::Status { job_id } => {
            let resp = client
                .get(format!("{base}/api/v1/jobs/{job_id}"))
                .send()
                .await?;
            print_response(resp, cli.format).await?;
        }
        Command::List => {
            let resp = client
                .get(format!("{base}/api/v1/jobs"))
                .send()
                .await?;
            print_response(resp, cli.format).await?;
        }
        Command::Cancel { job_id } => {
            let resp = client
                .post(format!("{base}/api/v1/jobs/{job_id}/cancel"))
                .send()
                .await?;
            print_response(resp, cli.format).await?;
        }
        Command::Health => {
            let resp = client
                .get(format!("{base}/health/ready"))
                .send()
                .await?;
            print_response(resp, cli.format).await?;
        }
    }

    Ok(())
}

#[cfg(feature = "cli")]
async fn print_response(
    resp: reqwest::Response,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = resp.status();
    let body: serde_json::Value = resp.json().await?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        OutputFormat::Text => {
            if status.is_success() {
                print_text_value(&body, 0);
            } else {
                let msg = body
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                eprintln!("Error ({}): {}", status, msg);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "cli")]
fn print_text_value(value: &serde_json::Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                match v {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        println!("{pad}{k}:");
                        print_text_value(v, indent + 2);
                    }
                    _ => println!("{pad}{k}: {}", format_scalar(v)),
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                println!("{pad}[{i}]:");
                print_text_value(v, indent + 2);
            }
        }
        other => println!("{pad}{}", format_scalar(other)),
    }
}

#[cfg(feature = "cli")]
fn format_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn test_parse_submit() {
        let cli = Cli::try_parse_from(["flume-cli", "submit", "my-job"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Submit {
                job_name,
                parallelism: None
            } if job_name == "my-job"
        ));
    }

    #[test]
    fn test_parse_submit_with_parallelism() {
        let cli =
            Cli::try_parse_from(["flume-cli", "submit", "my-job", "--parallelism", "4"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Submit {
                job_name,
                parallelism: Some(4)
            } if job_name == "my-job"
        ));
    }

    #[test]
    fn test_parse_status() {
        let cli = Cli::try_parse_from(["flume-cli", "status", "abc-123"]).unwrap();
        assert!(matches!(cli.command, Command::Status { job_id } if job_id == "abc-123"));
    }

    #[test]
    fn test_parse_list() {
        let cli = Cli::try_parse_from(["flume-cli", "list"]).unwrap();
        assert!(matches!(cli.command, Command::List));
    }

    #[test]
    fn test_parse_cancel() {
        let cli = Cli::try_parse_from(["flume-cli", "cancel", "abc-123"]).unwrap();
        assert!(matches!(cli.command, Command::Cancel { job_id } if job_id == "abc-123"));
    }

    #[test]
    fn test_parse_health() {
        let cli = Cli::try_parse_from(["flume-cli", "health"]).unwrap();
        assert!(matches!(cli.command, Command::Health));
    }

    #[test]
    fn test_custom_endpoint() {
        let cli = Cli::try_parse_from([
            "flume-cli",
            "--endpoint",
            "http://localhost:9090",
            "health",
        ])
        .unwrap();
        assert_eq!(cli.endpoint, "http://localhost:9090");
    }

    #[test]
    fn test_json_format() {
        let cli = Cli::try_parse_from(["flume-cli", "--format", "json", "list"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Json));
    }
}
