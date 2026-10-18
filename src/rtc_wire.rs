//! Bounded framing on an already authenticated private QUIC control stream.
use iroh::endpoint::{RecvStream, SendStream};
use serde_json::Value;
use std::io;

pub const MAX_CONTROL_FRAME: usize = 65536;

pub async fn write_frame(send: &mut SendStream, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_CONTROL_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RTC control frame too large",
        ));
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    send.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_optional_frame(
    recv: &mut RecvStream,
    maximum: usize,
) -> io::Result<Option<Value>> {
    let mut header = [0; 4];
    let mut position = 0;
    while position < header.len() {
        match recv
            .read(&mut header[position..])
            .await
            .map_err(io::Error::other)?
        {
            None if position == 0 => return Ok(None),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "RTC control header incomplete",
                ));
            }
            Some(length) => position += length,
        }
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > maximum.min(MAX_CONTROL_FRAME) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RTC control frame exceeds bound",
        ));
    }
    let mut body = vec![0; length];
    recv.read_exact(&mut body).await.map_err(io::Error::other)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

pub async fn read_frame(recv: &mut RecvStream) -> io::Result<Value> {
    read_optional_frame(recv, MAX_CONTROL_FRAME)
        .await?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "RTC private control stream ended",
            )
        })
}

/// The same 32-bit JSON frame on the constrained application-control channel.
pub async fn write_control(
    exchange: &iroh::endpoint::ControlExchange,
    value: &Value,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    exchange
        .send_message(
            iroh::endpoint::ControlMessageFormat::LengthPrefixed32 {
                maximum: MAX_CONTROL_FRAME,
            },
            &bytes,
        )
        .await
}
pub async fn read_control(exchange: &iroh::endpoint::ControlExchange) -> io::Result<Value> {
    let bytes = exchange
        .recv_message(iroh::endpoint::ControlMessageFormat::LengthPrefixed32 {
            maximum: MAX_CONTROL_FRAME,
        })
        .await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
