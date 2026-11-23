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

pub struct DataWriter(Writer);
pub struct DataReader(Reader);
enum Writer {
    Quic(SendStream),
    Local(Option<tokio::io::DuplexStream>),
}
enum Reader {
    Quic(RecvStream),
    Local(tokio::io::DuplexStream),
}

fn transport(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

impl DataStream {
    pub(crate) fn from_parts(send: SendStream, recv: RecvStream) -> Self {
        Self {
            send: DataWriter(Writer::Quic(send)),
            recv: DataReader(Reader::Quic(recv)),
        }
    }

    /// The application-facing record stream on a retained byte channel.
    pub(crate) fn local(
        capacity: usize,
    ) -> (Self, tokio::io::DuplexStream, tokio::io::DuplexStream) {
        let (send, read) = tokio::io::duplex(capacity);
        let (write, recv) = tokio::io::duplex(capacity);
        (
            Self {
                send: DataWriter(Writer::Local(Some(send))),
                recv: DataReader(Reader::Local(recv)),
            },
            read,
            write,
        )
    }
    pub(crate) fn stream_id(&self) -> Option<u64> {
        match &self.send.0 {
            Writer::Quic(stream) => Some(stream.id().into()),
            Writer::Local(_) => None,
        }
    }

    pub async fn open(connection: &Connection) -> io::Result<Self> {
        let (send, recv) = n0_future::time::timeout(IDLE, connection.open_bi())
            .await
            .map_err(transport)?
            .map_err(transport)?;
        Ok(Self::from_parts(send, recv))
    }

    pub async fn accept(connection: &Connection) -> io::Result<Self> {
        let (send, recv) = n0_future::time::timeout(IDLE, connection.accept_bi())
            .await
            .map_err(transport)?
            .map_err(transport)?;
        Ok(Self::from_parts(send, recv))
    }

    pub async fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send.write_chunk(bytes).await
    }

    pub async fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        self.recv.read_chunk().await
    }

    pub(crate) async fn write_channel_record(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.send.write_chunk_without_timeout(bytes).await
    }
    pub(crate) async fn read_channel_record(&mut self) -> io::Result<Vec<u8>> {
        self.recv
            .read_chunk_without_timeout()
            .await?
            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
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
        if matches!(self.0, Writer::Local(_)) {
            return self.write_chunk_without_timeout(bytes).await;
        }
        n0_future::time::timeout(IDLE, self.write_chunk_without_timeout(bytes))
            .await
            .map_err(transport)?
    }
    pub(crate) async fn write_chunk_without_timeout(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > CHUNK_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "record too large",
            ));
        }
        {
            let length = (bytes.len() as u32).to_be_bytes();
            match &mut self.0 {
                Writer::Quic(stream) => {
                    stream.write_all(&length).await.map_err(transport)?;
                    stream.write_all(bytes).await.map_err(transport)
                }
                Writer::Local(Some(stream)) => {
                    use tokio::io::AsyncWriteExt;
                    stream.write_all(&length).await?;
                    stream.write_all(bytes).await
                }
                Writer::Local(None) => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }
    }

    pub fn finish(&mut self) -> io::Result<()> {
        match &mut self.0 {
            Writer::Quic(stream) => stream.finish().map_err(transport),
            Writer::Local(stream) => {
                stream.take();
                Ok(())
            }
        }
    }
    pub fn cancel(&mut self) {
        match &mut self.0 {
            Writer::Quic(stream) => {
                let _ = stream.reset(1u32.into());
            }
            Writer::Local(stream) => {
                stream.take();
            }
        }
    }
}

impl DataReader {
    async fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match &mut self.0 {
            Reader::Quic(stream) => stream
                .read(bytes)
                .await
                .map(|n| n.unwrap_or(0))
                .map_err(transport),
            Reader::Local(stream) => {
                use tokio::io::AsyncReadExt;
                stream.read(bytes).await
            }
        }
    }
    async fn read_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        match &mut self.0 {
            Reader::Quic(stream) => stream.read_exact(bytes).await.map_err(transport),
            Reader::Local(stream) => {
                use tokio::io::AsyncReadExt;
                stream.read_exact(bytes).await.map(|_| ())
            }
        }
    }

    pub async fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        self.read_chunk_or_end()
            .await?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "application stream ended"))
    }

    /// Clean FIN is distinct from a truncated record or a reset. Duplex TCP
    /// consumers can half-close without suppressing actual transport failures.
    pub async fn read_chunk_or_end(&mut self) -> io::Result<Option<Vec<u8>>> {
        if matches!(self.0, Reader::Local(_)) {
            return self.read_chunk_without_timeout().await;
        }
        n0_future::time::timeout(IDLE, self.read_chunk_without_timeout())
            .await
            .map_err(transport)?
    }
    pub(crate) async fn read_chunk_without_timeout(&mut self) -> io::Result<Option<Vec<u8>>> {
        {
            let mut length = [0; 4];
            if self.read(&mut length[..1]).await? == 0 {
                return Ok(None);
            }
            self.read_exact(&mut length[1..]).await?;
            let length = u32::from_be_bytes(length) as usize;
            if length > CHUNK_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "record too large",
                ));
            }
            let mut bytes = vec![0; length];
            self.read_exact(&mut bytes).await?;
            Ok(Some(bytes))
        }
    }

    pub fn cancel(&mut self) {
        if let Reader::Quic(stream) = &mut self.0 {
            let _ = stream.stop(1u32.into());
        }
    }
}
