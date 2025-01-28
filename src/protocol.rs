use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};

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

impl<'de> EasyCodeRead<'de> for RecvStream {
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
