mod guarded_socket;
mod handshake;
mod protocol;
mod zellij;

use std::process::exit;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::{
    select,
    signal::{self, unix::SignalKind},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

#[derive(Debug, Parser)]
pub struct JoinArgs {
    #[arg(help = "Peer to peer Endpoint ID of the host you want to join")]
    host: String,
    #[arg(help = "Pre Shared Secret, also provided by the host")]
    secret: String,
}

#[derive(Parser, Debug)]
pub enum Command {
    Host,
    Join(JoinArgs),
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("zeco=info")
        .init();

    let args = Command::parse();
    let cancellation_token = CancellationToken::new();
    tokio::spawn(listen_for_shutdown(cancellation_token.clone()));
    let res = match args {
        Command::Host => handshake::handshake_host(&cancellation_token).await,
        Command::Join(args) => {
            handshake::handshake_guest(&args.host, &args.secret, &cancellation_token).await
        }
    };
    if let Err(e) = res {
        error!("Terminated due to: {e:#}");
        exit(1);
    }
}

async fn listen_for_shutdown(cancellation_token: CancellationToken) -> Result<()> {
    let mut sigterm_listener = signal::unix::signal(SignalKind::terminate())
        .context("Couldn't set up SIGTERM listener")?;

    select! {
        _ = signal::ctrl_c() => {},
        _ = sigterm_listener.recv() => {},
    }

    info!("Performing a graceful shutdown...");
    cancellation_token.cancel();
    Ok(())
}
