//! Member-authenticated device management messages on the existing data path.
use std::io;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    data_stream::DataStream,
    device_operation::{DeviceApproval, DeviceOperation},
    service_protocol::ServiceBundle,
};

#[derive(Serialize, Deserialize)]
pub enum DeviceRequest {
    Confirm { operation: DeviceOperation },
}

#[derive(Serialize, Deserialize)]
pub struct DeviceReply {
    pub approval: Option<DeviceApproval>,
}

#[derive(Serialize, Deserialize)]
pub struct ResultRequest {
    pub operation: p2panda_core::Hash,
}
#[derive(Serialize, Deserialize)]
pub struct ResultReply {
    pub approval: Option<DeviceApproval>,
    pub history: ServiceBundle,
}

pub async fn write_message(stream: &mut DataStream, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 64 * 1024 {
        return Err(io::Error::other("device message too large"));
    }
    stream.write_chunk(&bytes).await
}

pub async fn read_message<T: DeserializeOwned>(stream: &mut DataStream) -> io::Result<T> {
    Ok(serde_json::from_slice(&stream.read_chunk().await?)?)
}

/// The accompanying signed history has its own existing 16MiB bound.
pub async fn write_exchange(
    stream: &mut DataStream,
    value: &impl Serialize,
    history: &ServiceBundle,
) -> io::Result<()> {
    write_message(stream, value).await?;
    history.write(stream).await
}
