//! Complete invitation material, carried unchanged by text, links and QR hosts.
//! Contact hints are not authorization. No genesis seed or admission secret is
//! represented here; only a precode may contain its short-lived bearer token.
use std::io;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{EndpointAddr, RelayUrl};
use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{
    irc_datagram::IrcFacility, management_auth::NetworkAnchor, mqtt_datagram::MqttFacility,
};

pub const MAX_INVITATION_BYTES: usize = 32 * 1024;
const LINK: &str = "miopunch://invite/";

/// Explicit cross-platform mappings are preserved; URL similarity never invents
/// a native/WSS alias. Raw native-only facilities remain usable on native nodes.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapContact {
    pub relays: Vec<String>,
    pub mqtt: Vec<MqttFacility>,
    pub irc: Vec<IrcFacility>,
    pub native_signaling: Vec<String>,
}
impl BootstrapContact {
    pub fn validate(&self) -> io::Result<()> {
        if self.relays.len() > 16
            || self.mqtt.len() + self.irc.len() > 16
            || self.native_signaling.len() > 16
        {
            return Err(invalid());
        }
        for value in &self.relays {
            let url: RelayUrl = value.parse().map_err(|_| invalid())?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || url.query().is_some()
            {
                return Err(invalid());
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for facility in &self.mqtt {
            facility.validate()?;
            if !names.insert(&facility.logical_id) {
                return Err(invalid());
            }
        }
        for facility in &self.irc {
            facility.validate()?;
            if !names.insert(&facility.logical_id) {
                return Err(invalid());
            }
        }
        for value in &self.native_signaling {
            let url: RelayUrl = value.parse().map_err(|_| invalid())?;
            if !matches!(url.scheme(), "mqtt" | "mqtts" | "irc")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
                || url.query().is_some()
                || url.path() != "/"
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientInvitation {
    pub(crate) version: u32,
    pub(crate) invitation: String,
    pub(crate) address: EndpointAddr,
    pub(crate) precode: Option<String>,
    #[serde(default)]
    pub(crate) bootstrap: BootstrapContact,
}
impl ClientInvitation {
    pub fn new(
        material: [u8; 96],
        address: EndpointAddr,
        precode: Option<[u8; 128]>,
        bootstrap: BootstrapContact,
    ) -> io::Result<Self> {
        let value = Self {
            version: 2,
            invitation: hex::encode(material),
            address,
            precode: precode.map(hex::encode),
            bootstrap,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_INVITATION_BYTES * 2 {
            return Err(invalid());
        }
        let decoded;
        let bytes = if let Some(encoded) = std::str::from_utf8(bytes)
            .ok()
            .map(str::trim)
            .and_then(|s| s.strip_prefix(LINK))
        {
            decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
            decoded.as_slice()
        } else {
            bytes
        };
        if bytes.len() > MAX_INVITATION_BYTES {
            return Err(invalid());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        if bytes.len() > MAX_INVITATION_BYTES {
            return Err(invalid());
        }
        Ok(bytes)
    }
    /// One representation for copying, links and a host's QR encoder. This
    /// method does not create UI or claim every maximum-sized invite fits a QR.
    pub fn text(&self) -> io::Result<String> {
        Ok(format!("{LINK}{}", URL_SAFE_NO_PAD.encode(self.encode()?)))
    }
    pub fn material_bytes(&self) -> io::Result<[u8; 96]> {
        hex::decode(&self.invitation)
            .map_err(|_| invalid())?
            .try_into()
            .map_err(|_| invalid())
    }
    pub fn precode_bytes(&self) -> io::Result<Option<[u8; 128]>> {
        self.precode
            .as_ref()
            .map(|value| {
                hex::decode(value)
                    .map_err(|_| invalid())?
                    .try_into()
                    .map_err(|_| invalid())
            })
            .transpose()
    }
    pub fn anchor(&self) -> io::Result<NetworkAnchor> {
        let bytes = self.material_bytes()?;
        Ok(NetworkAnchor {
            root: VerifyingKey::from_bytes(&bytes[..32].try_into().unwrap())
                .map_err(|_| invalid())?,
            genesis: Hash::from_bytes(bytes[32..64].try_into().unwrap()),
        })
    }
    pub fn contact(&self) -> iroh::EndpointId {
        self.address.id
    }
    pub fn address(&self) -> &EndpointAddr {
        &self.address
    }
    pub fn bootstrap(&self) -> &BootstrapContact {
        &self.bootstrap
    }
    fn validate(&self) -> io::Result<()> {
        let bytes = self.material_bytes()?;
        self.anchor()?;
        let contact = iroh::EndpointId::from_bytes(&bytes[64..].try_into().unwrap())
            .map_err(|_| invalid())?;
        if !matches!(self.version, 1 | 2)
            || self.address.id != contact
            || self.address.addrs.len() > 32
            || self
                .precode_bytes()?
                .is_some_and(|code| code[..96] != bytes)
        {
            return Err(invalid());
        }
        self.bootstrap.validate()
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid complete invitation")
}
