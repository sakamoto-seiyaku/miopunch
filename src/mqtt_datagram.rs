//! Shared, platform-independent MQTT facility and MIO1 datagram contract.
//! Facility aliases are trusted local configuration, never broker-supplied hints.

use std::io;

use iroh_base::RelayUrl;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_DATAGRAM: usize = 2048;
pub const QUEUE_DEPTH: usize = 64;
pub const TRANSPORT_ID: u64 = 0x6d696f6d717474;
pub type Route = [u8; 16];
pub type Service = [u8; 16];

/// The same ACK qualification policy for native MQTT and browser MQTT-WSS.
#[derive(Debug, Default)]
pub struct Qualification {
    connected: bool,
    subscribe_id: Option<u16>,
    ready: bool,
}

impl Qualification {
    pub fn connack(&mut self, code: u8, session_present: bool) -> io::Result<()> {
        if self.connected || code != 0 || session_present {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "MQTT CONNACK rejected",
            ));
        }
        self.connected = true;
        Ok(())
    }

    pub fn subscribe(&mut self, packet_id: u16) -> io::Result<()> {
        if !self.connected || self.subscribe_id.is_some() || packet_id == 0 {
            return Err(invalid("unexpected MQTT subscription"));
        }
        self.subscribe_id = Some(packet_id);
        Ok(())
    }

    pub fn suback(&mut self, packet_id: u16, codes: &[u8]) -> io::Result<()> {
        if !self.connected || self.ready || Some(packet_id) != self.subscribe_id || codes != [0] {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "MQTT subscription rejected or unmatched SUBACK",
            ));
        }
        self.ready = true;
        Ok(())
    }

    pub fn connected(&self) -> bool {
        self.connected
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn retire(&mut self) {
        self.ready = false;
    }
}

pub fn encode_datagram(source: Route, bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_DATAGRAM {
        return Err(invalid("invalid MQTT datagram size"));
    }
    let mut frame = Vec::with_capacity(20 + bytes.len());
    frame.extend_from_slice(b"MIO1");
    frame.extend_from_slice(&source);
    frame.extend_from_slice(bytes);
    Ok(frame)
}

pub fn decode_datagram(frame: &[u8]) -> io::Result<(Route, &[u8])> {
    if !(21..=20 + MAX_DATAGRAM).contains(&frame.len()) || !frame.starts_with(b"MIO1") {
        return Err(invalid("invalid MIO1 MQTT frame"));
    }
    Ok((frame[4..20].try_into().unwrap(), &frame[20..]))
}

pub fn topic(route: Route) -> String {
    format!("miopunch/test/{}/packet", hex::encode(route))
}

pub fn address_for(service: Option<Service>, route: Route) -> iroh_base::CustomAddr {
    if let Some(service) = service {
        let mut data = [0; 32];
        data[..16].copy_from_slice(&service);
        data[16..].copy_from_slice(&route);
        iroh_base::CustomAddr::from_parts(TRANSPORT_ID, &data)
    } else {
        iroh_base::CustomAddr::from_parts(TRANSPORT_ID, &route)
    }
}

pub fn route_from_addr(
    addr: &iroh_base::CustomAddr,
    service: Option<Service>,
) -> io::Result<Route> {
    if addr.id() != TRANSPORT_ID {
        return Err(invalid("wrong MQTT carrier"));
    }
    let data = match service {
        Some(service) if addr.data().len() == 32 && addr.data()[..16] == service => {
            &addr.data()[16..]
        }
        Some(_) => return Err(invalid("MQTT route belongs to another service")),
        None => addr.data(),
    };
    data.try_into()
        .map_err(|_| invalid("invalid MQTT route length"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MqttFacility {
    pub logical_id: String,
    pub native_url: String,
    pub wss_url: String,
}

impl MqttFacility {
    pub fn new(logical_id: String, native_url: String, wss_url: String) -> io::Result<Self> {
        if logical_id.is_empty()
            || logical_id.len() > 128
            || logical_id.chars().any(char::is_control)
        {
            return Err(invalid("invalid MQTT logical facility"));
        }
        let native = entry(&native_url)?;
        let wss = entry(&wss_url)?;
        if !matches!(native.scheme(), "mqtt" | "mqtts")
            || native.path() != "/"
            || wss.scheme() != "wss"
        {
            return Err(invalid(
                "MQTT facility requires a native MQTT entry and a WSS entry",
            ));
        }
        Ok(Self {
            logical_id,
            native_url: native.to_string(),
            wss_url: wss.to_string(),
        })
    }

    /// Revalidate deserialized configuration before using it.
    pub fn validate(&self) -> io::Result<Self> {
        Self::new(
            self.logical_id.clone(),
            self.native_url.clone(),
            self.wss_url.clone(),
        )
    }

    pub fn service_tag(&self) -> io::Result<[u8; 16]> {
        self.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"miopunch/mqtt-facility/1\0");
        hash.update(self.logical_id.as_bytes());
        Ok(hash.finalize()[..16].try_into().unwrap())
    }

    /// Disabling either entry or the logical ID disables the entire facility.
    /// An alias cannot restore a facility disabled through its other entry.
    pub fn check_allowed(&self, disabled: &[String]) -> io::Result<()> {
        self.validate()?;
        let native = origin(&self.native_url)?;
        let wss = origin(&self.wss_url)?;
        if disabled.iter().any(|value| {
            value == &self.logical_id
                || origin(value).is_ok_and(|value| value == native || value == wss)
        }) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "MQTT facility disabled",
            ));
        }
        Ok(())
    }
}

fn entry(value: &str) -> io::Result<RelayUrl> {
    let url: RelayUrl = value
        .parse()
        .map_err(|_| invalid("invalid MQTT entry URL"))?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "MQTT entry URL must not contain credentials, query, or fragment",
        ));
    }
    Ok(url)
}

fn origin(value: &str) -> io::Result<String> {
    let url = entry(value)?;
    let port = url.port().unwrap_or(match url.scheme() {
        "mqtt" => 1883,
        "mqtts" => 8883,
        "wss" => 443,
        _ => return Err(invalid("invalid MQTT entry scheme")),
    });
    Ok(format!(
        "{}://{}:{port}",
        url.scheme(),
        url.host_str().unwrap().trim_end_matches('.')
    ))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
