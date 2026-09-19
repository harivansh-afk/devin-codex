use clap::{Parser, Subcommand};
use devin_codex::{config, gateway::GatewayClient, request::audit_request, server};
use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    version,
    about = "Experimental Responses adapter. Full Codex parity is not yet supported."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the verified protocol subset on loopback.
    Serve {
        #[arg(long, default_value_t = 19476, value_parser = clap::value_parser!(u16).range(1024..))]
        port: u16,
        #[arg(long)]
        token_file: Option<PathBuf>,
        #[arg(long)]
        credentials: Option<PathBuf>,
    },
    /// Check a JSON request without sending inference. Use - for stdin.
    Audit { request: PathBuf },
    /// Create a new private local token file. Never overwrites an existing file.
    Token { path: PathBuf },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<u8, &'static str> {
    match cli.command {
        Command::Token { path } => {
            config::write_token(&path)
                .map_err(|_| "Cannot create private token file; it may already exist.")?;
            println!("Created private token file.");
            Ok(0)
        }
        Command::Audit { request } => {
            let reader: Box<dyn Read> = if request.as_os_str() == "-" {
                Box::new(std::io::stdin())
            } else {
                Box::new(std::fs::File::open(request).map_err(|_| "Cannot read request file.")?)
            };
            let mut raw = Vec::new();
            reader
                .take((server::MAX_BODY + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|_| "Cannot read request.")?;
            if raw.len() > server::MAX_BODY {
                return Err("Request exceeds size limit.");
            }
            let body = serde_json::from_slice(&raw).map_err(|_| "Request must be valid JSON.")?;
            let problems = audit_request(&body);
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"supported":problems.is_empty(), "problems":problems})
                )
                .unwrap()
            );
            Ok(u8::from(!problems.is_empty()))
        }
        Command::Serve {
            port,
            token_file,
            credentials,
        } => {
            let credentials = config::load_credentials(credentials.as_deref()).map_err(
                |_| "Cannot load Devin credentials; check the file and deployment origin.",
            )?;
            let token = config::load_local_token(token_file.as_deref()).map_err(
                |_| "Cannot load local token; set DEVIN_CODEX_TOKEN or a private --token-file.",
            )?;
            let gateway = GatewayClient::new(&credentials.endpoint, &credentials.api_key)
                .map_err(|_| "Cannot configure Devin gateway transport.")?;
            let listener =
                tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                    .await
                    .map_err(|_| "Cannot bind loopback listener.")?;
            println!(
                "Serving verified subset on http://127.0.0.1:{port}/v1; full Codex parity is blocked."
            );
            axum::serve(listener, server::router(credentials, token, gateway))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .map_err(|_| "HTTP server failed.")?;
            Ok(0)
        }
    }
}
