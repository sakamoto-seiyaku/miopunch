//! Bounded message exchanges for an explicitly selected existing control wire.

use std::io;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{Connection, RecvStream, SendStream};

/// Framing of one complete application control message.
///
/// This describes the existing wire; it does not expose byte-stream I/O. The
/// library validates the length before allocating or reading the body. The
/// application remains responsible for validating the message's protocol meaning.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ControlMessageFormat {
    /// A two-byte big-endian length followed by at most `maximum` body bytes.
    LengthPrefixed16 {
        /// Maximum body size, at most 65,535 bytes.
        maximum: usize,
    },
    /// A four-byte big-endian length followed by a bounded body.
    LengthPrefixed32 {
        /// Maximum body size. Chosen by the application protocol, at most u32::MAX.
        maximum: usize,
    },
    /// Exactly `length` bytes, without a length field or FIN.
    Fixed {
        /// The application's exact message size, at most u32::MAX.
        length: usize,
    },
    /// A bounded body terminated by QUIC FIN, with no added framing bytes.
    EndTerminated {
        /// Maximum body size chosen by the application, at most u32::MAX.
        maximum: usize,
    },
}

impl ControlMessageFormat {
    fn maximum(self) -> io::Result<usize> {
        let maximum = match self {
            Self::LengthPrefixed16 { maximum } if maximum <= usize::from(u16::MAX) => maximum,
            Self::LengthPrefixed32 { maximum } => maximum,
            Self::Fixed { length } => length,
            Self::EndTerminated { maximum } => maximum,
            _ => return Err(io::Error::other("invalid control message length bound")),
        };
        if maximum == 0 || maximum > u32::MAX as usize {
            return Err(io::Error::other("invalid control message bound"));
        }
        Ok(maximum)
    }
}

#[derive(Debug)]
pub(super) struct ApplicationControl {
    retired: CancellationToken,
}

impl ApplicationControl {
    pub(super) fn new() -> Self {
        Self {
            retired: CancellationToken::new(),
        }
    }

    pub(super) fn cancel(&self) {
        self.retired.cancel();
    }

    pub(super) async fn open(&self, connection: &Connection) -> io::Result<ControlExchange> {
        let (send, recv) = tokio::select! { biased;
            _ = self.retired.cancelled() => return Err(retired()),
            result = connection.open_bi() => result.map_err(io::Error::other)?,
        };
        Ok(ControlExchange::new(send, recv, self.retired.clone()))
    }

    pub(super) async fn accept(&self, connection: &Connection) -> io::Result<ControlExchange> {
        let (send, recv) = tokio::select! { biased;
            _ = self.retired.cancelled() => return Err(retired()),
            result = connection.accept_bi() => result.map_err(io::Error::other)?,
        };
        Ok(ControlExchange::new(send, recv, self.retired.clone()))
    }
}

fn retired() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "application control exchange retired",
    )
}

/// An owned exchange of bounded complete control messages.
///
/// There are no stream/datagram getters, ordinary connection conversions, or
/// AsyncRead/AsyncWrite implementations. Business handoff retires all exchanges
/// from the pending control phase, including ones retained by the application.
#[derive(Debug)]
pub struct ControlExchange {
    stream_id: crate::endpoint::StreamId,
    send: Mutex<Option<SendStream>>,
    recv: Mutex<Option<RecvStream>>,
    retired: CancellationToken,
}

impl ControlExchange {
    /// Narrow streams already owned by an ordinary caller to bounded messages.
    ///
    /// This does not authenticate or admit a connection. It lets an existing
    /// control implementation share its framing with the restricted interface;
    /// the consumed streams cannot be extracted again.
    pub fn from_streams(send: SendStream, recv: RecvStream) -> Self {
        Self::new(send, recv, CancellationToken::new())
    }

    fn new(send: SendStream, recv: RecvStream, retired: CancellationToken) -> Self {
        Self {
            stream_id: send.id(),
            send: Mutex::new(Some(send)),
            recv: Mutex::new(Some(recv)),
            retired,
        }
    }

    /// Identity of the original private exchange, for continuity diagnostics.
    pub fn id(&self) -> crate::endpoint::StreamId {
        self.stream_id
    }

    /// Encode one complete bounded message in the application's existing format.
    /// End-terminated messages also finish this exchange's sending direction.
    pub async fn send_message(
        &self,
        format: ControlMessageFormat,
        message: &[u8],
    ) -> io::Result<()> {
        let maximum = format.maximum()?;
        if message.len() > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control message exceeds its bound",
            ));
        }
        if matches!(format, ControlMessageFormat::Fixed { length } if message.len() != length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control message has the wrong fixed size",
            ));
        }
        let mut guard = self.send.lock().await;
        let send = guard.as_mut().ok_or_else(retired)?;
        let work = async {
            if matches!(format, ControlMessageFormat::LengthPrefixed16 { .. }) {
                send.write_all(&(message.len() as u16).to_be_bytes())
                    .await
                    .map_err(io::Error::other)?;
            } else if matches!(format, ControlMessageFormat::LengthPrefixed32 { .. }) {
                send.write_all(&(message.len() as u32).to_be_bytes())
                    .await
                    .map_err(io::Error::other)?;
            }
            send.write_all(message).await.map_err(io::Error::other)?;
            if matches!(format, ControlMessageFormat::EndTerminated { .. }) {
                send.finish().map_err(io::Error::other)?;
            }
            Ok(())
        };
        let result = tokio::select! { biased;
            _ = self.retired.cancelled() => Err(retired()),
            result = work => result,
        };
        if result.is_err() {
            let _ = send.reset(0u32.into());
        }
        if result.is_err() || matches!(format, ControlMessageFormat::EndTerminated { .. }) {
            guard.take();
        }
        result
    }

    /// Receive one complete message, checking its bound before reading a body.
    /// Cancelling a partial message invalidates that exchange; drop it to end it.
    pub async fn recv_message(&self, format: ControlMessageFormat) -> io::Result<Vec<u8>> {
        let maximum = format.maximum()?;
        let mut guard = self.recv.lock().await;
        let recv = guard.as_mut().ok_or_else(retired)?;
        let work = async {
            match format {
                ControlMessageFormat::LengthPrefixed16 { .. } => {
                    let mut header = [0; 2];
                    recv.read_exact(&mut header)
                        .await
                        .map_err(io::Error::other)?;
                    let length = usize::from(u16::from_be_bytes(header));
                    if length > maximum {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "control message exceeds its bound",
                        ));
                    }
                    let mut message = vec![0; length];
                    recv.read_exact(&mut message)
                        .await
                        .map_err(io::Error::other)?;
                    Ok(message)
                }
                ControlMessageFormat::LengthPrefixed32 { .. } => {
                    let mut header = [0; 4];
                    recv.read_exact(&mut header)
                        .await
                        .map_err(io::Error::other)?;
                    let length = u32::from_be_bytes(header) as usize;
                    if length > maximum {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "control message exceeds its bound",
                        ));
                    }
                    let mut message = Vec::new();
                    message
                        .try_reserve_exact(length)
                        .map_err(io::Error::other)?;
                    message.resize(length, 0);
                    recv.read_exact(&mut message)
                        .await
                        .map_err(io::Error::other)?;
                    Ok(message)
                }
                ControlMessageFormat::Fixed { length } => {
                    let mut message = Vec::new();
                    message
                        .try_reserve_exact(length)
                        .map_err(io::Error::other)?;
                    message.resize(length, 0);
                    recv.read_exact(&mut message)
                        .await
                        .map_err(io::Error::other)?;
                    Ok(message)
                }
                ControlMessageFormat::EndTerminated { .. } => {
                    recv.read_to_end(maximum).await.map_err(io::Error::other)
                }
            }
        };
        let result = tokio::select! { biased;
            _ = self.retired.cancelled() => Err(retired()),
            result = work => result,
        };
        if result.is_err() {
            let _ = recv.stop(0u32.into());
        }
        if result.is_err() || matches!(format, ControlMessageFormat::EndTerminated { .. }) {
            guard.take();
        }
        result
    }

    /// Finish the sending direction after the application's last complete message.
    pub async fn finish_send(&self) -> io::Result<()> {
        if self.retired.is_cancelled() {
            return Err(retired());
        }
        let mut guard = self.send.lock().await;
        let mut send = guard.take().ok_or_else(retired)?;
        send.finish().map_err(io::Error::other)
    }
    /// Finish complete messages and wait for the peer's transport acknowledgement.
    /// Retirement cancels the wait; no stream capability is exposed.
    pub async fn finish_send_confirmed(&self) -> io::Result<()> {
        if self.retired.is_cancelled() {
            return Err(retired());
        }
        let mut send = self.send.lock().await.take().ok_or_else(retired)?;
        send.finish().map_err(io::Error::other)?;
        tokio::select! {biased;
            _=self.retired.cancelled()=>Err(retired()),
            result=send.stopped()=>result.map(|_|()).map_err(io::Error::other),
        }
    }
}

impl Drop for ControlExchange {
    fn drop(&mut self) {
        if let Some(mut send) = self.send.get_mut().take() {
            let _ = send.reset(0u32.into());
        }
        if let Some(mut recv) = self.recv.get_mut().take() {
            let _ = recv.stop(0u32.into());
        }
    }
}
