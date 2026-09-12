use std::{io::ErrorKind, path::PathBuf};

use anyhow::{bail, Context, Result};
use tokio::net::{unix::SocketAddr, UnixListener, UnixStream};
use tracing::warn;

pub struct GuardedSocket {
    listener: Option<UnixListener>,
    path: PathBuf,
}

impl GuardedSocket {
    pub async fn accept(&self) -> Result<(UnixStream, SocketAddr)> {
        Ok(self.listener.as_ref().unwrap().accept().await?)
    }

    pub async fn bind(path: PathBuf) -> Result<GuardedSocket> {
        Self::remove_if_stale(&path).await?;

        let listener = UnixListener::bind(&path).context(format!(
            "Failed to create socket file at {}.",
            path.display()
        ))?;

        Ok(GuardedSocket {
            listener: Some(listener),
            path,
        })
    }

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    async fn remove_if_stale(path: &PathBuf) -> Result<()> {
        // Check to see if a socket already exists and is live
        let err = match UnixStream::connect(&path).await {
            Ok(_) => {
                // success means the socket is live, so something else is using it
                bail!("Another process is using the live socket at {} -- is another zeco client already running and connected to the same session?", path.display())
            }
            Err(e) => e,
        };

        match err.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => {}
            _ => {
                bail!(
                    "Couldn't check whether socket file already exists and is live: {}",
                    err
                )
            }
        };

        // socket file is stale/dangling symlink/nonexistent, safe to remove
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() == ErrorKind::NotFound {
                // file doesn't exist, carry on
            } else {
                bail!(
                    "Couldn't cleanup an existing socket file before attempting connection: {}",
                    e
                )
            }
        };

        Ok(())
    }
}

impl Drop for GuardedSocket {
    fn drop(&mut self) {
        // Ensure we close file descriptor by dropping listener before unlinking
        drop(self.listener.take());
        let result = std::fs::remove_file(&self.path);
        if let Err(err) = result {
            warn!("Failed to remove socket file during cleanup: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::exists;
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn bind_handles_stale_socket_files() -> Result<()> {
        let dir = tempdir()?;
        let stale_socket_path = dir.path().join("zeco-test-socket");
        let stale_socket = UnixListener::bind(&stale_socket_path)
            .context("Couldn't bind to the test socket path to create a stale socket.")?;
        drop(stale_socket); // closes file descriptor, but file remains
        GuardedSocket::bind(stale_socket_path)
            .await
            .context("Couldn't bind to stale socket at test_socket_path.")?;
        Ok(())
    }

    #[tokio::test]
    async fn bind_doesnt_remove_live_sockets() -> Result<()> {
        let dir = tempdir()?;
        let live_socket_path = dir.path().join("zeco-test-socket");
        let _live_socket = UnixListener::bind(&live_socket_path)
            .context("Couldn't bind to the test socket path to create a live socket.")?;
        assert!(GuardedSocket::bind(live_socket_path.clone()).await.is_err());
        assert!(exists(live_socket_path).is_ok_and(|file_exists| file_exists));
        Ok(())
    }

    #[tokio::test]
    async fn guarded_socket_gets_cleaned_up() -> Result<()> {
        let dir = tempdir()?;
        let guarded_socket_path = dir.path().join("zeco-test-socket");
        let guarded_socket = GuardedSocket::bind(guarded_socket_path.clone()).await?;
        assert!(exists(&guarded_socket_path).is_ok_and(|file_exists| file_exists));
        drop(guarded_socket);
        assert!(exists(guarded_socket_path).is_ok_and(|file_exists| !file_exists));
        Ok(())
    }
}
