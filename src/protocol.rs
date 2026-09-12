use core::fmt;
use std::{fmt::Display, str::FromStr};

use anyhow::{anyhow, Result};
use iroh::endpoint::{RecvStream, SendStream};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{io::copy, net::UnixStream};

pub const ALPN: &[u8] = &[3, 1, 4, 1, 5, 9, 2, 6];

pub trait EasyCodeWrite {
    async fn struct_write<T: Serialize>(&mut self, t: &T) -> Result<()>;
}

pub trait EasyCodeRead<'de> {
    async fn struct_read<T: DeserializeOwned>(&mut self) -> Result<T>;
}

impl EasyCodeWrite for SendStream {
    async fn struct_write<T: Serialize>(&mut self, t: &T) -> Result<()> {
        let v: Vec<u8> = bincode::serialize(t).unwrap();
        let size = v.len() as u32;
        self.write_all(&size.to_be_bytes()).await?;
        self.write_all(&v).await?;
        Ok(())
    }
}

impl EasyCodeRead<'_> for RecvStream {
    async fn struct_read<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut length_bytes = [0; 4];
        self.read_exact(&mut length_bytes).await?;
        let length = u32::from_be_bytes(length_bytes);
        let mut v = vec![0; length as usize];
        self.read_exact(&mut v).await?;
        let t: T = bincode::deserialize(&v)?;
        Ok(t)
    }
}

#[derive(PartialEq, Clone, Debug)]
pub struct PreSharedKey([u8; Self::LEN]);

impl PreSharedKey {
    pub const LEN: usize = 32;

    pub fn generate() -> PreSharedKey {
        let mut rng = thread_rng();
        PreSharedKey(std::array::from_fn(|_| rng.sample(Alphanumeric)))
    }

    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl FromStr for PreSharedKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes: [u8; Self::LEN] = s
            .as_bytes()
            .try_into()
            .map_err(|_| anyhow!("psk must be exactly {} bytes, got {}", Self::LEN, s.len()))?;

        Ok(PreSharedKey(bytes))
    }
}

impl Display for PreSharedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = str::from_utf8(&self.0).expect("Psk bytes are valid UTF-8");
        f.write_str(s)
    }
}

// Helper to shuffle bytes between zellij socket and iroh connection
pub async fn proxy(
    mut iroh_send: SendStream,
    mut iroh_recv: RecvStream,
    mut stream: UnixStream,
) -> Result<()> {
    let (mut socket_read, mut socket_write) = stream.split();

    let a = copy(&mut socket_read, &mut iroh_send);
    let b = copy(&mut iroh_recv, &mut socket_write);

    let (a, b) = tokio::join!(a, b);
    a?;
    b?;
    Ok(())
}
