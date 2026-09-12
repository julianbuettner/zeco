use std::{
    env::{self, temp_dir},
    fs::read_dir,
    path::PathBuf,
    process,
};

use anyhow::{bail, Result};
use directories::ProjectDirs;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct ZellijSessionInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
}

pub fn get_base_path() -> Result<PathBuf> {
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
        path: socket_file,
        version: version.to_string_lossy().to_string(),
        name: session_name,
    })
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
