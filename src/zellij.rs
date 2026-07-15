use std::{
    env::{self, temp_dir},
    fs::read_dir,
    path::PathBuf,
    process,
};

use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::{fs::create_dir_all, io::copy, net::UnixStream, spawn, task::spawn_blocking};
use tracing::{error, info};

use crate::{
    guarded_socket::GuardedSocket,
    protocol::{EasyCodeRead, EasyCodeWrite},
};
use tokio_util::sync::CancellationToken;

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

pub async fn host(
    c: Connection,
    z: ZellijSessionInfo,
    cancellation_token: &CancellationToken,
) -> Result<()> {
    let mut s = c.open_uni().await?;
    s.struct_write(&z.version).await?;
    s.struct_write(&z.name).await?;
    loop {
        let z = z.clone();
        tokio::select! {
            x = c.accept_bi() => {
                match x {
                    Ok((send, recv)) => {
                        spawn(handle_zellij_session(send, recv, z));
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

pub async fn join(c: Connection, cancellation_token: &CancellationToken) -> Result<()> {
    let mut s = c.accept_uni().await?;
    let version: String = s.struct_read().await?;
    let name: String = s.struct_read().await?;
    info!("Remote session is '{name}'. You too are expected to use version {version}.");

    let dir = get_base_path()?.join(version);
    create_dir_all(&dir)
        .await
        .context("Failed to create zellij directory")?;
    let remote_session_name = format!("{name}-remote");
    let local_socket_path = dir.join(&remote_session_name);
    let guarded_socket = GuardedSocket::bind(local_socket_path).await?;
    let _t = spawn_blocking(|| attach_zellij(remote_session_name));
    loop {
        tokio::select! {
            result = guarded_socket.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let c = c.clone();
                        spawn(handle_zellij_socket(stream, c));
                    }
                    Err(_) => error!("Failed to accept connection on socket."),
                }
            }
            _ = cancellation_token.cancelled() => {
                    return Ok(())
            }
        }
    }
}

pub fn attach_zellij(session_name: String) {
    let mut p = process::Command::new("zellij");
    p.arg("attach").arg(&session_name);
    let mut handle = match p.spawn() {
        Err(e) => {
            error!("Failed to spawn `zellij attach {session_name}`: {e}");
            return;
        }
        Ok(v) => v,
    };
    let done = handle.wait();
    if let Err(e) = done {
        error!("zellij quit with an error: {e}");
    }
    info!("Connection still open. Rejoin with `zellij a {session_name}` or quit with Ctrl+C.");
}
