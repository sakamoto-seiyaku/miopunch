//! Bounded application records on an admitted SDK stream, on Native and WASM.
//!
//! Each operation carries at most one 64 KiB record. QUIC supplies backpressure;
//! callers do not enqueue an entire file or retain an unbounded output queue.
use std::{io, time::Duration};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};

pub const CHUNK_BYTES: usize = 64 * 1024;
const IDLE: Duration = Duration::from_secs(60);
const CONTROL_BYTES: usize = 4096;

pub struct DataStream {
    send: DataWriter,
    recv: DataReader,
}

pub struct DataWriter(SendStream);
pub struct DataReader(RecvStream);

fn transport(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

impl DataStream {
    pub async fn open(connection: &Connection) -> io::Result<Self> {
        let (send, recv) = n0_future::time::timeout(IDLE, connection.open_bi())
            .await
            .map_err(transport)?
            .map_err(transport)?;
        Ok(Self {
            send: DataWriter(send),
            recv: DataReader(recv),
        })
    }

    pub async fn accept(connection: &Connection) -> io::Result<Self> {
        let (send, recv) = n0_future::time::timeout(IDLE, connection.accept_bi())
            .await
            .map_err(transport)?
            .map_err(transport)?;
        Ok(Self {
            send: DataWriter(send),
            recv: DataReader(recv),
        })
    }

    pub async fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send.write_chunk(bytes).await
    }

    pub async fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        self.recv.read_chunk().await
    }

    /// Independent read/write ownership permits duplex consumers to await a
    /// read while writing, without introducing a runtime queue between them.
    pub fn split(self) -> (DataWriter, DataReader) {
        (self.send, self.recv)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.send.finish()
    }

    pub fn cancel(&mut self) {
        self.send.cancel();
        self.recv.cancel();
    }

    pub(crate) async fn write_control(&mut self, value: &impl Serialize) -> io::Result<()> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > CONTROL_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control record too large",
            ));
        }
        self.write_chunk(&bytes).await
    }

    pub(crate) async fn read_control<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        let bytes = self.read_chunk().await?;
        if bytes.len() > CONTROL_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control record too large",
            ));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}

impl DataWriter {
    pub async fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > CHUNK_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record too large",
            ));
        }
        n0_future::time::timeout(IDLE, async {
            self.0
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .map_err(transport)?;
            self.0.write_all(bytes).await.map_err(transport)
        })
        .await
        .map_err(transport)?
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.0.finish().map_err(transport)
    }
    pub fn cancel(&mut self) {
        let _ = self.0.reset(1u32.into());
    }
}

impl DataReader {
    pub async fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        self.read_chunk_or_end().await?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "application stream ended"))
    }

    /// Clean FIN is distinct from a truncated record or a reset. Duplex TCP
    /// consumers can half-close without suppressing actual transport failures.
    pub async fn read_chunk_or_end(&mut self) -> io::Result<Option<Vec<u8>>> {
        n0_future::time::timeout(IDLE, async {
            let mut length = [0; 4];
            if self.0.read(&mut length[..1]).await.map_err(transport)?.is_none() {
                return Ok(None);
            }
            self.0.read_exact(&mut length[1..]).await.map_err(transport)?;
            let length = u32::from_be_bytes(length) as usize;
            if length > CHUNK_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "record too large",
                ));
            }
            let mut bytes = vec![0; length];
            self.0.read_exact(&mut bytes).await.map_err(transport)?;
            Ok(Some(bytes))
        })
        .await
        .map_err(transport)?
    }

    pub fn cancel(&mut self) {
        let _ = self.0.stop(1u32.into());
    }
}
