// Our goal is to establish two iroh::Connections
// for the host and the guest.

use anyhow::{bail, Result};
use iroh::{endpoint::Incoming, Endpoint, NodeId, SecretKey};
use rand::{distributions::Alphanumeric, rngs::OsRng, thread_rng, Rng};
use std::str::FromStr;

use crate::zellij::{self, get_current_session};

const ALPN: &[u8] = &[3, 1, 4, 1, 5, 9, 2, 6];

async fn init_endpoint() -> Result<Endpoint> {
    let secret_key = SecretKey::generate(OsRng);
    Endpoint::builder()
        .secret_key(secret_key)
        .discovery_n0()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
}

pub async fn handshake_host() -> Result<()> {
    let zellij_info = get_current_session()?;
    println!(
        "Sharing Zellij session {} (version {})",
        zellij_info.name, zellij_info.version
    );

    let psk: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let endpoint = init_endpoint().await?;
    println!("The client now can join with the following command:");
    println!("\tzeco join {} {}", endpoint.node_id(), psk);
    println!(
        "WARNING! Everyone with these credentials can execute arbitrary commands in your shell. \
        Only hand over to people you fully trust."
    );
    println!("Waiting for guest to join. Press Ctrl-C to quit.");

    let incoming: Incoming = endpoint.accept().await.unwrap();
    let connection = incoming.accept()?.await?;
    println!("Connection established.");

    let (mut send, mut recv) = connection.accept_bi().await?;
    assert_eq!(psk.len(), 32); // String::length is in bytes
    let mut buf = [0; 32];
    recv.read_exact(&mut buf).await?;
    if buf != psk.as_bytes() {
        send.write_all(&[0]).await?;
        bail!("Client provided wrong secret. Quit.");
    }
    send.write_all(&[1]).await?;
    send.finish()?;
    println!("Client authenticated successfully!");
    drop(send);
    drop(recv);

    zellij::host(connection, zellij_info).await
}

pub async fn handshake_guest(node_id: &str, secret: &str) -> Result<()> {
    let node_id: NodeId = NodeId::from_str(node_id)?;
    let endpoint = init_endpoint().await?;

    let connection = endpoint.connect(node_id, ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;
    send.write_all(secret.as_bytes()).await?;
    send.finish()?;
    let mut success = [0];
    recv.read_exact(&mut success).await?;
    if success != [1] {
        bail!("Host declined provided secret.");
    }
    println!("Host let you in.");
    drop(send);
    drop(recv);

    zellij::join(connection).await
}
