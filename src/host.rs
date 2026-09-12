use anyhow::{bail, Context, Result};
use iroh::{
    endpoint::{Connection, Incoming},
    Endpoint,
};
use tokio::{net::UnixStream, spawn};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    protocol::{proxy, EasyCodeWrite, PreSharedKey},
    zellij::ZellijSessionInfo,
};

#[derive(Debug)]
pub struct Host {
    // We store the endpoint in the struct to keep it alive. When it goes out of scope, iroh
    // machinery is shut down
    _endpoint: Endpoint,
    connection: Connection,
    session_info: ZellijSessionInfo,
}

impl Host {
    pub async fn accept(
        endpoint: Endpoint,
        session_info: ZellijSessionInfo,
        psk: &PreSharedKey,
    ) -> Result<Host> {
        let incoming: Incoming = endpoint
            .accept()
            .await
            .context("endpoint closed before a guest connected")?;
        let connection = incoming.accept()?.await?;
        info!("Connection established.");

        let (mut send, mut recv) = connection.accept_bi().await?;
        let mut buf = [0; PreSharedKey::LEN];
        recv.read_exact(&mut buf).await?;
        if buf != *psk.as_bytes() {
            send.write_all(&[0]).await?;
            bail!("Guest provided wrong secret. Quit.");
        }
        send.write_all(&[1]).await?;
        send.finish()?;
        info!("Guest authenticated successfully!");
        drop(send);
        drop(recv);

        let mut s = connection.open_uni().await?;
        s.struct_write(&session_info.version).await?;
        s.struct_write(&session_info.name).await?;
        s.finish()?;

        Ok(Host {
            _endpoint: endpoint,
            session_info,
            connection,
        })
    }

    pub async fn serve(&self, cancellation_token: CancellationToken) -> Result<()> {
        loop {
            tokio::select! {
                x = self.connection.accept_bi() => {
                    match x {
                        Ok((send, recv)) => {
                            let z = self.session_info.clone();
                            spawn(async move {
                                let stream = UnixStream::connect(z.path).await?;
                                proxy(send, recv, stream).await?;
                                Ok::<(), anyhow::Error>(())
                            });
                        }
                        Err(e) => bail!("Failed to accept channel from guest: {:?}", e),
                    }
                }
                _ = cancellation_token.cancelled() => {
                    return Ok(())
                }
            }
        }
    }
}
