use std::{path::PathBuf, time::Duration};

use anyhow::{bail, Context, Result};
use iroh::endpoint::presets;
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
    try_join,
};
use tokio_util::sync::CancellationToken;

use crate::{
    guest::Guest, host::Host, init_endpoint, protocol::PreSharedKey, zellij::ZellijSessionInfo,
};

// This performs host and guest side setups and sends bytes across the wire, confirming that
// traffic makes it across guest_socket -> guest_iroh_node -> host_iroh_node -> host_socket and
// can be read back out.
#[tokio::test]
async fn host_and_guest_complete_roundtrip() -> Result<()> {
    let cancellation_token = CancellationToken::new();
    let session_name = "test-session";

    // Zellij normally creates and owns the host-side socket, but we'll make a temp placeholder for testing
    let host_socket_dir = tempdir()?;
    let host_socket_path = host_socket_dir.path().join(session_name);

    let guest_socket_dir = tempdir()?;

    let session_info = ZellijSessionInfo {
        path: host_socket_path.clone(),
        version: String::from("contract_version_1"),
        name: String::from(session_name),
    };

    let host_endpoint = init_endpoint(presets::Minimal)
        .await
        .context("Failed to bind host endpoint")?;
    let host_addr = host_endpoint.addr();

    let guest_endpoint = init_endpoint(presets::Minimal)
        .await
        .context("Failed to bind guest endpoint")?;
    let psk = PreSharedKey::generate();

    let (host, guest) = try_join!(
        Host::accept(host_endpoint, session_info, &psk),
        Guest::connect(
            guest_endpoint,
            guest_socket_dir.path().to_path_buf(),
            host_addr,
            &psk,
        )
    )
    .context("Failed to prepare either the host or guest")?;

    // tokio::select! on the host task, the guest task, and a validation task
    // host and guest tasks both block forever while they proxy zellij session data, only returning
    // when the cancellation token gets cancelled. Our third task can actually send data across the
    // wire and validate it, and return a success/cancel the other two
    tokio::select! {
        res = host.serve(cancellation_token.clone()) => bail!("host task ended unexpectedly: {res:?}"),
        res = guest.serve(cancellation_token.clone()) => bail!("guest task ended unexpectedly: {res:?}"),
        res = simulate_zellij_traffic(host_socket_path, guest.socket_path()) => {
            if let Err(e) = res {
                bail!("zellij traffic simulation ended unexpectedly: {e:?}")
            };
            Ok(())
        }
    }
}

async fn simulate_zellij_traffic(
    host_socket_path: PathBuf,
    guest_socket_path: PathBuf,
) -> Result<()> {
    let timeout_limit = Duration::from_secs(5);
    let guest_input = b"6.28318530";

    // zellij normally owns the host-side socket, but in testing we need to simulate the host
    // socket existing and listening
    let host_listener = UnixListener::bind(&host_socket_path)?;

    // UnixListener::accept() will only resolve when something connects to it. This won't happen
    // until we forward traffic from the host-side iroh node, as the QUIC stream is only
    // visible to the host peer once bytes are written to it. So we simultaneously listen on
    // the host socket while connecting the guest stream and writing bytes.
    let (host_result, guest_result) = tokio::try_join!(
        async {
            host_listener
                .accept()
                .await
                .context("Fake zellij server never received a connection")
        },
        async {
            let mut guest_stream = timeout(timeout_limit, UnixStream::connect(guest_socket_path))
                .await
                .context("Timed out connecting to guest's test socket")?
                .context("Failed to connect to guest's test socket")?;

            guest_stream
                .write_all(guest_input)
                .await
                .context("Failed to write bytes to guest test socket")?;

            Ok::<_, anyhow::Error>(guest_stream)
        },
    )?;

    let (mut host_stream, _) = host_result;

    let mut host_output_buffer = vec![0; guest_input.len()];
    host_stream
        .read_exact(&mut host_output_buffer)
        .await
        .context("Failed to read out bytes from host test socket")?;

    assert_eq!(guest_input, &host_output_buffer.as_slice());

    // Write some bytes back the other way (host -> guest)
    let host_input = b"2.71828182";
    let mut guest_stream = guest_result;
    host_stream
        .write_all(host_input)
        .await
        .context("Failed to write bytes to host test socket")?;

    let mut guest_output_buffer = vec![0; host_input.len()];
    guest_stream
        .read_exact(&mut guest_output_buffer)
        .await
        .context("Failed to read out bytes from guest socket")?;

    assert_eq!(host_input, &guest_output_buffer.as_slice());

    Ok(())
}
