use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use iroh::{endpoint::Connection, Endpoint, EndpointAddr};
use tokio::{fs::create_dir_all, spawn};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    guarded_socket::GuardedSocket,
    protocol::{proxy, EasyCodeRead, PreSharedKey, ALPN},
};

pub struct Guest {
    connection: Connection,
    // We store the endpoint in the struct to keep it alive. When it goes out of scope, iroh
    // machinery is shut down
    _endpoint: Endpoint,
    remote_session_name: String,
    socket: GuardedSocket,
}

impl Guest {
    pub async fn connect(
        endpoint: Endpoint,
        zellij_base_path: PathBuf,
        host_endpoint_addr: impl Into<EndpointAddr>,
        psk: &PreSharedKey,
    ) -> Result<Guest> {
        let connection = endpoint.connect(host_endpoint_addr, ALPN).await?;
        let (mut send, mut recv) = connection.open_bi().await?;
        send.write_all(psk.as_bytes()).await?;
        send.finish()?;
        let mut success = [0];
        recv.read_exact(&mut success).await?;
        if success != [1] {
            bail!("Host declined provided secret.");
        }
        info!("Host let you in.");
        drop(send);
        drop(recv);

        let mut stream = connection.accept_uni().await?;
        let version: String = stream.struct_read().await?;
        let name: String = stream.struct_read().await?;
        info!("Remote session is '{name}'. You too are expected to use version {version}.");

        let dir = zellij_base_path.join(version);
        create_dir_all(&dir)
            .await
            .context("Failed to create zellij directory")?;
        let remote_session_name = format!("{name}-remote");
        let local_socket_path = dir.join(&remote_session_name);
        let socket = GuardedSocket::bind(local_socket_path).await?;

        Ok(Guest {
            connection,
            _endpoint: endpoint,
            remote_session_name,
            socket,
        })
    }

    pub async fn serve(&self, cancellation_token: CancellationToken) -> Result<()> {
        loop {
            tokio::select! {
                result = self.socket.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let connection = self.connection.clone();
                            // We never await the JoinHandle, so the task's Result would be dropped unseen;
                            // we should refactor this if those errors are important to capture
                            // outside logs emitted within proxy()
                            spawn(async move {
                                let (send, recv) = connection.open_bi().await?;
                                proxy(send, recv, stream).await?;
                                Ok::<(), anyhow::Error>(())
                            });
                        }
                        Err(e) => error!("Failed to accept connection on socket, {}", e),
                    }
                }
                _ = cancellation_token.cancelled() => {
                    return Ok(())
                }
            }
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn socket_path(&self) -> PathBuf {
        self.socket.path()
    }

    pub fn session_name(&self) -> String {
        self.remote_session_name.clone()
    }
}
