use std::path::PathBuf;

use grpc_web_server::server::{Server, ServerOptions};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt};

use clap::{Parser, ValueEnum};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The address to host the HTTP/1.1 proxy/web server on.
    #[arg(long, default_value_t = String::from("127.0.0.1:80"))]
    http_address: String,
    /// The address of the gRPC server to forwarded the requests to and from
    #[arg(long, default_value_t = String::from("127.0.0.1:50051"))]
    grpc_address: String,
    /// The directory of the static web files to host
    #[arg(long)]
    static_dir: Option<String>,
    /// Logging verbosity level
    #[arg(long, value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
    /// The path to the CA cert used to generate the gRPC server and proxy certificates and keys. Required for TLS.
    #[arg(long)]
    grpc_ca_cert: Option<PathBuf>,
    /// The path to the gRPC proxy private key. Required for mTLS
    #[arg(long)]
    grpc_proxy_key: Option<PathBuf>,
    /// The path to the gRPC proxy certification. Required for mTLS
    #[arg(long)]
    grpc_proxy_cert: Option<PathBuf>,
}

impl Into<ServerOptions> for Args {
    fn into(self) -> ServerOptions {
        ServerOptions {
            http_address: self.http_address,
            grpc_address: self.grpc_address,
            static_dir: self.static_dir,
            grpc_ca_cert: self.grpc_ca_cert,
            grpc_proxy_key: self.grpc_proxy_key,
            grpc_proxy_cert: self.grpc_proxy_cert,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let filter = EnvFilter::new(format!(
        "grpc_web_server={},tower_http=info",
        args.log_level.as_str()
    ));

    fmt().with_env_filter(filter).init();

    info!(
        http_address = %args.http_address,
        grpc_address = %args.grpc_address,
        static_dir = ?args.static_dir,
        "Starting grpc-web-server"
    );

    let server = Server::new(args.into())?;
    if let Err(err) = server.start().await {
        error!(error = %err, "Server exited with error");
        return Err(err);
    }

    Ok(())
}
