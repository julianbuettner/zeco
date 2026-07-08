use std::{
    env::{self, temp_dir},
    fs::read_dir,
    io::ErrorKind,
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::{
    fs::create_dir_all,
    io::copy,
    net::{unix::SocketAddr, UnixListener, UnixStream},
    spawn,
};

use crate::protocol::{EasyCodeRead, EasyCodeWrite};

#[derive(Debug, Clone)]
pub struct ZellijSessionInfo {
    pub name: String,
    pub version: String,
    pub path: String,
}

fn get_base_path() -> Result<PathBuf> {
    // Env override
    if let Ok(p) = env::var("ZELLIJ_SOCKET_DIR") {
        return Ok(p.into());
    }

    // Linux
    let zellij_proj_dir = ProjectDirs::from("org", "Zellij Contributors", "Zellij");
    if zellij_proj_dir.is_none() {
        bail!("Unexpected environment. Your OS platform is not supported. Please open a issue on GitHub.")
    }
    let zellij_proj_dir = zellij_proj_dir.unwrap();
    if let Some(d) = zellij_proj_dir.runtime_dir() {
        return Ok(d.into());
    }

    // Mac OS / special Unix
    let uid = nix::unistd::Uid::current();
    let zellij_tmp_dir: PathBuf = temp_dir().join(format!("zellij-{uid}"));

    Ok(zellij_tmp_dir)
}

pub fn get_current_session() -> Result<ZellijSessionInfo> {
    let zellij_base_path = get_base_path()?;
    let session_name = env::var("ZELLIJ_SESSION_NAME");
    if session_name.is_err() {
        bail!(
            "Could not find ZELLIJ_SESSION_NAME in environment. \
            Please run this command from within your active zellij session."
        )
    }
    let session_name = session_name.unwrap();

    let mut socket_file = None;
    let mut version = None;
    for entry in read_dir(&zellij_base_path)? {
        let entry = entry?;
        let mut potential_socket_file: PathBuf = entry.path();
        potential_socket_file.push(&session_name);
        if potential_socket_file.exists() {
            socket_file = Some(potential_socket_file);
            version = Some(entry.file_name());
            break;
        }
    }
    if socket_file.is_none() {
        bail!("Could not find the socket for your current zellij session. This is a bug.");
    }
    let socket_file = socket_file.unwrap();
    let version = version.unwrap();

    Ok(ZellijSessionInfo {
        path: socket_file.to_string_lossy().to_string(),
        version: version.to_string_lossy().to_string(),
        name: session_name,
    })
}

pub async fn host(c: Connection, z: ZellijSessionInfo) -> Result<()> {
    let mut s = c.open_uni().await?;
    s.struct_write(&z.version).await?;
    s.struct_write(&z.name).await?;
    println!("Sent zellij details");
    loop {
        let z = z.clone();
        let x = c.accept_bi().await;
        match x {
            Ok((send, recv)) => {
                spawn(handle_zellij_session(send, recv, z));
            }
            Err(e) => bail!("Failed to accept channel from guest: {:?}", e),
        }
    }
}

async fn handle_zellij_session(
    mut send: SendStream,
    mut recv: RecvStream,
    z: ZellijSessionInfo,
) -> Result<()> {
    let mut u = UnixStream::connect(z.path).await?;
    let (mut socket_read, mut socket_write) = u.split();

    let a = copy(&mut socket_read, &mut send);
    let b = copy(&mut recv, &mut socket_write);

    let (a, b) = tokio::join!(a, b);
    a?;
    b?;
    Ok(())
}

async fn handle_zellij_socket(mut socket_stream: UnixStream, c: Connection) -> Result<()> {
    let (mut iroh_send, mut iroh_recv) = c.open_bi().await?;
    let (mut sock_read, mut sock_write) = socket_stream.split();

    let a = copy(&mut sock_read, &mut iroh_send);
    let b = copy(&mut iroh_recv, &mut sock_write);

    let (a, b) = tokio::join!(a, b);
    a?;
    b?;
    Ok(())
}

struct GuardedSocket {
    listener: Option<UnixListener>,
    path: PathBuf,
}

impl GuardedSocket {
    async fn accept(&self) -> Result<(UnixStream, SocketAddr)> {
        Ok(self.listener.as_ref().unwrap().accept().await?)
    }

    async fn bind(path: PathBuf) -> Result<GuardedSocket> {
        Self::remove_if_stale(&path).await?;

        let listener = UnixListener::bind(&path).context(format!(
            "Failed to create socket file at {}.",
            &path.display()
        ))?;

        Ok(GuardedSocket {
            listener: Some(listener),
            path,
        })
    }

    async fn remove_if_stale(path: &PathBuf) -> Result<()> {
        // Check to see if a socket already exists and is live
        let err = match UnixStream::connect(&path).await {
            Ok(_) => {
                // success means the socket is live, so something else is using it
                bail!("Another process is using the live socket at {} -- is another zeco client already running and connected to the same session?", &path.display())
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
            println!(
                "Warning: Failed to remove socket file during cleanup: {}",
                err
            )
        }
    }
}

pub async fn join(c: Connection) -> Result<()> {
    let mut s = c.accept_uni().await?;
    let version: String = s.struct_read().await?;
    let name: String = s.struct_read().await?;
    println!("Remote Session is {name}. You too are expected to use version {version}.");

    let dir = get_base_path()?.join(version);
    create_dir_all(&dir)
        .await
        .context("Failed to create zellij directory")?;
    let remote_session_name = format!("{name}-remote");
    let local_socket_path = dir.join(&remote_session_name);
    let guarded_socket = GuardedSocket::bind(local_socket_path).await?;
    println!("Join session with");
    println!("\tzellij a {remote_session_name}");
    loop {
        match guarded_socket.accept().await {
            Ok((stream, _)) => {
                let c = c.clone();
                spawn(handle_zellij_socket(stream, c));
            }
            Err(_) => println!("Failed to accept connection on socket."),
        }
    }
}
