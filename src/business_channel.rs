//! Application IO retained independently of a member stream's lifetime.
use std::{io, time::Duration};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    data_stream::DataStream, forwarding::ChannelRequest, native_member_services::ServiceTransport,
};

// Reuse the existing service lifetime for this first slice. Resource defaults
// and per-node accounting are selected by MIOPUNCH-173.
pub(crate) const RECOVERY: Duration = Duration::from_secs(60);

impl ChannelRequest {
    pub(crate) fn new() -> Self {
        Self {
            id: hex::encode(rand::random::<[u8; 16]>()),
            resume: false,
        }
    }
}

pub(crate) enum ChannelError {
    NetworkInterrupted,
    Business(io::Error),
}

fn carrier_error(transport: &ServiceTransport, error: io::Error) -> ChannelError {
    if transport.network_interrupted() {
        ChannelError::NetworkInterrupted
    } else {
        ChannelError::Business(error)
    }
}

/// The original application's halves outlive each attached transport. This
/// bridge is shared application machinery and has no dial or path scheduler.
pub(crate) struct BusinessChannel<R, W> {
    read: R,
    write: W,
    sent_fin: bool,
    received_fin: bool,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> BusinessChannel<R, W> {
    pub(crate) fn new(read: R, write: W) -> Self {
        Self {
            read,
            write,
            sent_fin: false,
            received_fin: false,
        }
    }

    pub(crate) async fn carry(
        &mut self,
        stream: DataStream,
        transport: &ServiceTransport,
    ) -> Result<(), ChannelError> {
        let (mut send, mut recv) = stream.split();
        let upload = async {
            if self.sent_fin {
                send.finish()
                    .map_err(|error| carrier_error(transport, error))?;
                return Ok::<_, ChannelError>(());
            }
            let mut bytes = vec![0; crate::data_stream::CHUNK_BYTES];
            loop {
                let count = self
                    .read
                    .read(&mut bytes)
                    .await
                    .map_err(ChannelError::Business)?;
                if count == 0 {
                    self.sent_fin = true;
                    send.finish()
                        .map_err(|error| carrier_error(transport, error))?;
                    return Ok(());
                }
                send.write_chunk(&bytes[..count])
                    .await
                    .map_err(|error| carrier_error(transport, error))?;
            }
        };
        let download = async {
            if self.received_fin {
                return Ok::<_, ChannelError>(());
            }
            while let Some(bytes) = recv
                .read_chunk_or_end()
                .await
                .map_err(|error| carrier_error(transport, error))?
            {
                self.write
                    .write_all(&bytes)
                    .await
                    .map_err(ChannelError::Business)?;
            }
            self.received_fin = true;
            self.write.shutdown().await.map_err(ChannelError::Business)
        };
        tokio::select! {
            result = async { tokio::try_join!(upload, download)?; Ok(()) } => result,
            _ = transport.closed() => Err(carrier_error(transport, io::Error::new(io::ErrorKind::ConnectionAborted, "business carrier ended"))),
        }
    }
}
