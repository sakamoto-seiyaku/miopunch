//! The original wire sends the TLS device key. The named form puts a marker in
//! that slot and appends the authenticated device's signed metadata.
use std::io;

use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use iroh_base::EndpointId;
use p2panda_core::VerifyingKey;

use crate::device_name::DeviceName;

const NAMED_DEVICE: [u8; 32] = [0xff; 32];
const NAME_FRAME: ControlMessageFormat = ControlMessageFormat::LengthPrefixed16 { maximum: 512 };

pub(crate) struct AdmissionRequestWire {
    pub(crate) binding: [u8; 112],
    pub(crate) applicant: VerifyingKey,
    pub(crate) token: Option<[u8; 32]>,
    pub(crate) name: Option<DeviceName>,
}

pub(crate) async fn read(
    exchange: &ControlExchange,
    remote: EndpointId,
    precode: bool,
) -> io::Result<AdmissionRequestWire> {
    let wire = exchange
        .recv_message(ControlMessageFormat::Fixed { length: 144 })
        .await?;
    let key = &wire[112..];
    if key != remote.as_bytes() && key != NAMED_DEVICE {
        return Err(invalid());
    }
    let applicant = VerifyingKey::from_bytes(remote.as_bytes()).map_err(io::Error::other)?;
    let token = if precode {
        let token = exchange
            .recv_message(ControlMessageFormat::Fixed { length: 32 })
            .await?;
        Some(token.try_into().unwrap())
    } else {
        None
    };
    let name = if key == NAMED_DEVICE {
        let bytes = exchange.recv_message(NAME_FRAME).await?;
        let name: DeviceName = p2panda_core::cbor::decode_cbor(bytes.as_slice())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if name.device != applicant || !name.valid() {
            return Err(invalid());
        }
        Some(name)
    } else {
        None
    };
    Ok(AdmissionRequestWire {
        binding: wire[..112].try_into().unwrap(),
        applicant,
        token,
        name,
    })
}

pub(crate) async fn write(
    exchange: &ControlExchange,
    binding: &[u8; 112],
    applicant: VerifyingKey,
    token: Option<[u8; 32]>,
    name: Option<&DeviceName>,
) -> io::Result<()> {
    let mut wire = binding.to_vec();
    wire.extend_from_slice(if name.is_some() {
        &NAMED_DEVICE
    } else {
        applicant.as_bytes()
    });
    if let Some(token) = token {
        wire.extend_from_slice(&token);
    }
    exchange
        .send_message(ControlMessageFormat::Fixed { length: wire.len() }, &wire)
        .await?;
    if let Some(name) = name {
        let bytes = p2panda_core::cbor::encode_cbor(name).map_err(io::Error::other)?;
        exchange.send_message(NAME_FRAME, &bytes).await?;
    }
    Ok(())
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "invalid admission device identity",
    )
}
