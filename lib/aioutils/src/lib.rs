use std::{pin::Pin, time::Duration};

use serde::{de::DeserializeOwned, Serialize};
use smol::{channel::Receiver, prelude::*};

mod dns;
pub use dns::*;

/// Reads a bincode-deserializable value with a 16bbe length
pub async fn read_pascalish<T: DeserializeOwned>(
    reader: &mut (impl AsyncRead + Unpin),
) -> anyhow::Result<T> {
    // first read 2 bytes as length
    let mut len_bts = [0u8; 2];
    reader.read_exact(&mut len_bts).await?;
    let len = u16::from_be_bytes(len_bts);
    // then read len
    let mut true_buf = vec![0u8; len as usize];
    reader.read_exact(&mut true_buf).await?;
    // then deserialize
    Ok(bincode::deserialize(&true_buf)?)
}

/// Writes a bincode-serializable value with a 16bbe length
pub async fn write_pascalish<T: Serialize>(
    writer: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> anyhow::Result<()> {
    let serialized = bincode::serialize(value).unwrap();
    assert!(serialized.len() <= 65535);
    // write bytes
    writer
        .write_all(&(serialized.len() as u16).to_be_bytes())
        .await?;
    writer.write_all(&serialized).await?;
    Ok(())
}

/// Copies an AsyncRead to an AsyncWrite, with a callback for every write.
#[inline]
pub async fn copy_with_stats(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    mut on_write: impl FnMut(usize),
) -> std::io::Result<()> {
    let mut buffer = [0u8; 16384];
    let mut timeout = smol::Timer::after(Duration::from_secs(300));
    loop {
        // first read into the small buffer
        let n = reader
            .read(&mut buffer)
            .or(async {
                (&mut timeout).await;
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "copy_with_stats timeout",
                ))
            })
            .await?;
        if n == 0 {
            return Ok(());
        }
        timeout.set_after(Duration::from_secs(300));
        on_write(n);
        writer
            .write_all(&buffer[..n])
            .or(async {
                (&mut timeout).await;
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "copy_with_stats timeout",
                ))
            })
            .await?;
    }
}

/// Copies an Read to an Write, with a callback for every write.
pub fn copy_with_stats_sync(
    mut reader: impl std::io::Read,
    mut writer: impl std::io::Write,
    mut on_write: impl FnMut(usize),
) -> std::io::Result<()> {
    let mut buffer = [0u8; 32768];
    loop {
        // first read into the small buffer
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            return Ok(());
        }
        on_write(n);
        writer.write_all(&buffer[..n])?;
    }
}

pub trait AsyncRW: AsyncRead + AsyncWrite {}

impl<T: AsyncRead + AsyncWrite> AsyncRW for T {}

pub type ConnLike = async_dup::Arc<async_dup::Mutex<Pin<Box<dyn AsyncRW + 'static + Send>>>>;

pub fn connify<T: AsyncRead + AsyncWrite + 'static + Send>(conn: T) -> ConnLike {
    async_dup::Arc::new(async_dup::Mutex::new(Box::pin(conn)))
}

pub fn to_ioerror<T: Into<Box<dyn std::error::Error + Send + Sync>>>(e: T) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e)
}

/// Reads from an async_channel::Receiver, but returns a vector of all available items instead of just one to save on context-switching.
pub async fn recv_chan_many<T>(ch: Receiver<T>) -> Result<Vec<T>, smol::channel::RecvError> {
    let mut toret = Vec::with_capacity(1);
    toret.push(ch.recv().await?);
    // push as many as possible
    while let Ok(val) = ch.try_recv() {
        toret.push(val);
    }
    Ok(toret)
}
