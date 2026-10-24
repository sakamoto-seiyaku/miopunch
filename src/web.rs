//! The Web service's bounded records, shared by Native and Browser clients.
//!
//! A target origin is provider configuration. It grants no member or service
//! permission; the existing signed service ledger remains authoritative.
use crate::{
    data_stream::DataStream,
    service_policy::{Service, ServiceAccess},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use p2panda_core::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::io;

pub const WEB_ALPN: &[u8] = b"miopunch/web/1";
pub const MAX_BODY: usize = 8 * 1024 * 1024;
pub const MAX_REQUEST_BODY: usize = 1024 * 1024;
pub const MAX_HEADERS: usize = 32 * 1024;
pub const MAX_SOCKET_MESSAGE: usize = 32 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebRequest {
    pub url: String,
    #[serde(default = "get")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default, with = "body_base64")]
    pub body: Vec<u8>,
}
fn get() -> String {
    "GET".into()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebResponse {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    #[serde(with = "body_base64")]
    pub body: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestHead {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub socket: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseHead {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub fn peer_permitted(access: &ServiceAccess, peer: VerifyingKey) -> bool {
    access.member(access.local) && access.member(peer)
        || access
            .guest_pairs_for(Service::Web)
            .iter()
            .any(|(id, _)| id == peer.as_bytes())
}

pub(crate) async fn write_head(stream: &mut DataStream, head: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(head)?;
    if bytes.len() > MAX_HEADERS {
        return Err(invalid("Web headers too large"));
    }
    stream.write_chunk(&bytes).await
}
pub(crate) async fn read_head<T: serde::de::DeserializeOwned>(
    stream: &mut DataStream,
) -> io::Result<T> {
    let bytes = stream.read_chunk().await?;
    if bytes.len() > MAX_HEADERS {
        return Err(invalid("Web headers too large"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) async fn write_body(
    stream: &mut DataStream,
    body: &[u8],
    maximum: usize,
) -> io::Result<()> {
    if body.len() > maximum {
        return Err(invalid("Web body too large"));
    }
    for chunk in body.chunks(crate::data_stream::CHUNK_BYTES) {
        stream.write_chunk(chunk).await?;
    }
    stream.write_chunk(&[]).await
}
pub(crate) async fn read_body(stream: &mut DataStream, maximum: usize) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let chunk = stream.read_chunk().await?;
        if chunk.is_empty() {
            return Ok(body);
        }
        if body.len() + chunk.len() > maximum {
            return Err(invalid("Web body too large"));
        }
        body.extend(chunk);
    }
}
pub async fn request(
    connection: &iroh::endpoint::Connection,
    request: WebRequest,
) -> io::Result<WebResponse> {
    let mut stream = DataStream::open(connection).await?;
    write_head(
        &mut stream,
        &RequestHead {
            url: request.url,
            method: request.method,
            headers: request.headers,
            socket: false,
        },
    )
    .await?;
    write_body(&mut stream, &request.body, MAX_REQUEST_BODY).await?;
    stream.finish()?;
    let head: ResponseHead = read_head(&mut stream).await?;
    let body = read_body(&mut stream, MAX_BODY).await?;
    Ok(WebResponse {
        url: head.url,
        status: head.status,
        headers: head.headers,
        body,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WebSocketMessage {
    Text {
        text: String,
    },
    Binary {
        #[serde(with = "body_base64")]
        data: Vec<u8>,
    },
    Close {
        code: u16,
        reason: String,
    },
}
impl WebSocketMessage {
    pub fn validate(&self) -> io::Result<()> {
        let length = match self {
            Self::Text { text } => text.len(),
            Self::Binary { data } => data.len(),
            Self::Close { code, reason } => {
                if !matches!(code, 1000..=1015 | 3000..=4999) || reason.len() > 123 {
                    return Err(invalid("invalid WebSocket close"));
                }
                2 + reason.len()
            }
        };
        if length > MAX_SOCKET_MESSAGE {
            return Err(invalid("WebSocket message too large"));
        }
        Ok(())
    }
    pub fn is_close(&self) -> bool {
        matches!(self, Self::Close { .. })
    }
}
pub(crate) async fn write_socket_message(
    writer: &mut crate::data_stream::DataWriter,
    message: &WebSocketMessage,
) -> io::Result<()> {
    message.validate()?;
    let mut record = Vec::new();
    match message {
        WebSocketMessage::Text { text } => {
            record.push(1);
            record.extend_from_slice(text.as_bytes());
        }
        WebSocketMessage::Binary { data } => {
            record.push(2);
            record.extend_from_slice(data);
        }
        WebSocketMessage::Close { code, reason } => {
            record.push(8);
            record.extend_from_slice(&code.to_be_bytes());
            record.extend_from_slice(reason.as_bytes());
        }
    }
    writer.write_chunk(&record).await
}
pub(crate) async fn read_socket_message(
    reader: &mut crate::data_stream::DataReader,
) -> io::Result<WebSocketMessage> {
    let record = reader.read_chunk().await?;
    if record.len() > MAX_SOCKET_MESSAGE + 1 {
        return Err(invalid("WebSocket message too large"));
    }
    let message = match record.split_first() {
        Some((1, data)) => WebSocketMessage::Text {
            text: String::from_utf8(data.to_vec())
                .map_err(|_| invalid("invalid WebSocket text"))?,
        },
        Some((2, data)) => WebSocketMessage::Binary {
            data: data.to_vec(),
        },
        Some((8, data)) if data.len() >= 2 => WebSocketMessage::Close {
            code: u16::from_be_bytes([data[0], data[1]]),
            reason: String::from_utf8(data[2..].to_vec())
                .map_err(|_| invalid("invalid WebSocket close reason"))?,
        },
        _ => return Err(invalid("invalid WebSocket message")),
    };
    message.validate()?;
    Ok(message)
}
pub(crate) async fn open_socket(
    connection: &iroh::endpoint::Connection,
    url: String,
    headers: Vec<(String, String)>,
) -> io::Result<(DataStream, ResponseHead)> {
    let mut stream = DataStream::open(connection).await?;
    write_head(
        &mut stream,
        &RequestHead {
            url,
            method: "GET".into(),
            headers,
            socket: true,
        },
    )
    .await?;
    write_body(&mut stream, &[], MAX_REQUEST_BODY).await?;
    let ready: ResponseHead = read_head(&mut stream).await?;
    if ready.status != 101 {
        return Err(invalid("WebSocket target did not upgrade"));
    }
    Ok((stream, ready))
}
pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

mod body_base64 {
    use super::*;
    pub fn serialize<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() > MAX_BODY.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("Web body too large"));
        }
        STANDARD.decode(value).map_err(serde::de::Error::custom)
    }
}
