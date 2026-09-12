mod guarded_socket;
mod guest;
mod host;
mod protocol;
mod zellij;

// For larger integration-like tests that don't belong strictly to one module
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use clap::Parser;
use iroh::{
    endpoint::presets::{self, Preset},
    Endpoint, EndpointId, SecretKey,
};
use tokio::{
    select,
    signal::{self, unix::SignalKind},
    task::spawn_blocking,
};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    guest::Guest,
    host::Host,
    protocol::{PreSharedKey, ALPN},
    zellij::{attach_zellij, get_base_path, get_current_session},
};

#[derive(Debug, Parser)]
pub struct JoinArgs {
    #[arg(help = "Peer to peer Endpoint ID of the host you want to join")]
    host: EndpointId,
    #[arg(help = "Pre Shared Secret, also provided by the host")]
    secret: PreSharedKey,
}

#[derive(Parser, Debug)]
pub enum Command {
    Host,
    Join(JoinArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("zeco=info")
        .init();

    let args = Command::parse();
    let cancellation_token = CancellationToken::new();
    tokio::spawn(listen_for_shutdown(cancellation_token.clone()));

    let zellij_base_path = get_base_path()?;

    let endpoint = async {
        init_endpoint(presets::N0)
            .await
            .context("Failed to bind endpoint")
    };

    match args {
        Command::Host => {
            let session_info = get_current_session()?;
            let psk = PreSharedKey::generate();
            let endpoint = endpoint.await?;

            println!(
                "Sharing Zellij session '{}' (version {})\n\
                 The guest can join with:\n\
                 \tzeco join {} {}\n\
                 WARNING! Everyone with these credentials can execute arbitrary commands in your shell. \
                 Only hand over to people you fully trust.\n\
                 Waiting for guest to join. Press Ctrl-C to quit.",
                session_info.name, session_info.version, endpoint.id(), psk
            );

            let host = Host::accept(endpoint, session_info, &psk).await?;
            host.serve(cancellation_token).await
        }

        Command::Join(args) => {
            let guest =
                Guest::connect(endpoint.await?, zellij_base_path, args.host, &args.secret).await?;

            let session_name = guest.session_name();
            let _attach = spawn_blocking(|| attach_zellij(session_name));
            guest.serve(cancellation_token).await
        }
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

async fn init_endpoint(preset: impl Preset) -> Result<Endpoint> {
    let secret_key = SecretKey::generate();
    Endpoint::builder(preset)
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind iroh endpoint: {e}"))
}
