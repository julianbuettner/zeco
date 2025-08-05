use std::{
    env,
    fs::{read_dir, DirEntry},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::{
    fs::create_dir_all,
    io::copy,
    net::{UnixListener, UnixStream},
    spawn,
};
use users::get_current_uid;

use crate::protocol::{EasyCodeRead, EasyCodeWrite};

#[derive(Debug, Clone)]
pub struct ZellijSessionInfo {
    pub name: String,
    pub version: String,
    pub path: String,
}

fn base_path() -> Result<PathBuf> {
    let mut p: PathBuf = env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not defined.")
        .map(Into::into)?;
    p.push("zellij");
    Ok(p)
}

pub fn get_current_session() -> Result<ZellijSessionInfo> {
    let zellij_base_path = base_path()?;
    let session_name = env::var("ZELLIJ_SESSION_NAME");
    if session_name.is_err() {
        bail!(
            "Could not find ZELLIJ_SESSION_NAME in environment. \
            Please run this command from within your active zellij session."
        )
    }
    let session_name = session_name.unwrap();
    let mut version_dirs: Vec<DirEntry> = Vec::new();
    for entry in read_dir(&zellij_base_path)? {
        version_dirs.push(entry?);
    }
    if version_dirs.is_empty() {
        bail!("Directory {:?} was empty unexpectedly.", zellij_base_path);
    }
    if version_dirs.len() > 1 {
        bail!(
            "Multiple directories found, where one was expected: {:?}",
            version_dirs
        );
    }
    let mut path = version_dirs.pop().unwrap().path();
    let version = path.file_name().unwrap().to_string_lossy().to_string();
    path.push(&session_name);
    if !std::fs::exists(&path)? {
        bail!("Expected file {} to exist.", path.to_string_lossy());
    }
    Ok(ZellijSessionInfo {
        path: path.to_string_lossy().to_string(),
        version,
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

pub async fn join(c: Connection) -> Result<()> {
    let mut s = c.accept_uni().await?;
    let version: String = s.struct_read().await?;
    let name: String = s.struct_read().await?;
    println!(
        "Remote Session is {}. You too are expected to use version {}.",
        name, version
    );

    let remote_session_name = format!("{}-remote", name);
    let dir = format!("/run/user/{}/zellij/{}", get_current_uid(), version);
    create_dir_all(&dir)
        .await
        .context("Failed to create zellij directory")?;
    let local_socket = format!("{}/{}", dir, remote_session_name);
    let listener = UnixListener::bind(local_socket).context("Failed to create socket file.")?;
    println!("Join session with");
    println!("\tzellij a {}", remote_session_name);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let c = c.clone();
                spawn(handle_zellij_socket(stream, c));
            }
            Err(_) => println!("Failed to accept connection on socket."),
        }
    }
}
