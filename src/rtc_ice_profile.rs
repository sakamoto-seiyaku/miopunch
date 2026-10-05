//! Explicit bounded ICE observation/candidate profile for the experimental actor.
//! Empty preserves the existing no-server profile; this never configures TURN.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, net::IpAddr, net::SocketAddr};

const MAX_OBSERVERS: usize = 2;
const MAX_ADDRESSES: usize = 8;
const MAX_SDP_BYTES: usize = 65536;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RtcIceProfile {
    #[serde(default)]
    pub stun_servers: Vec<String>,
    /// Native socket binding addresses for this attempt, independent of the
    /// candidate publication filter. Empty keeps the library's wildcard binding.
    /// Each explicit address binds an ephemeral UDP port; no TURN is enabled.
    #[serde(default)]
    pub local_bind_addresses: Vec<IpAddr>,
    /// Optional finite address publication restriction, applied after gathering.
    /// Local base addresses may remain in stats and related-address attributes.
    #[serde(default)]
    pub candidate_addresses: Vec<IpAddr>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn observer_address(value: &str) -> Option<SocketAddr> {
    let (scheme, remainder) = value.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("stun") {
        return None;
    }
    let resource: iroh::RelayUrl = format!(
        "stun://{}",
        remainder.strip_prefix("//").unwrap_or(remainder)
    )
    .parse()
    .ok()?;
    if !resource.username().is_empty() || resource.password().is_some() {
        return None;
    }
    Some(SocketAddr::new(
        resource.host_str()?.parse().ok()?,
        resource.port().unwrap_or(3478),
    ))
}

impl RtcIceProfile {
    pub fn validate(&self) -> io::Result<()> {
        if self.stun_servers.len() > MAX_OBSERVERS
            || self.candidate_addresses.len() > MAX_ADDRESSES
            || self.local_bind_addresses.len() > MAX_ADDRESSES
        {
            return Err(invalid(
                "RTC observation/candidate profile exceeds its bounds",
            ));
        }
        let mut bindings = BTreeSet::new();
        for address in &self.local_bind_addresses {
            if !address.is_ipv4()
                || address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || matches!(address, IpAddr::V4(value) if value.is_link_local())
                || !bindings.insert(*address)
            {
                return Err(invalid("RTC local binding address is invalid"));
            }
        }
        let mut observers = BTreeSet::new();
        for url in &self.stun_servers {
            // This first explicit experiment uses numeric IPv4 observers only.
            // No credentials, DNS aliases, TURN or implicit public resources.
            let address: SocketAddr = url
                .strip_prefix("stun:")
                .ok_or_else(|| invalid("RTC profile accepts only explicit STUN observers"))?
                .parse()
                .map_err(|_| invalid("RTC observer must be a numeric address and port"))?;
            if !address.is_ipv4()
                || address.port() == 0
                || address.ip().is_unspecified()
                || address.ip().is_multicast()
                || !observers.insert(address)
            {
                return Err(invalid("RTC observer address or duplicate is invalid"));
            }
        }
        let mut addresses = BTreeSet::new();
        for address in &self.candidate_addresses {
            if !address.is_ipv4()
                || address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || matches!(address, IpAddr::V4(value) if value.is_link_local())
                || !addresses.insert(*address)
            {
                return Err(invalid("RTC candidate publication address is invalid"));
            }
        }
        if !self.candidate_addresses.is_empty() && self.stun_servers.is_empty() {
            return Err(invalid(
                "restricted RTC publication requires an explicit observer",
            ));
        }
        Ok(())
    }

    pub(crate) fn native_bind_addresses(&self) -> Vec<SocketAddr> {
        if self.local_bind_addresses.is_empty() {
            vec![SocketAddr::from(([0, 0, 0, 0], 0))]
        } else {
            self.local_bind_addresses
                .iter()
                .map(|address| SocketAddr::new(*address, 0))
                .collect()
        }
    }

    pub fn check_allowed(&self, disabled: &[String]) -> io::Result<()> {
        self.validate()?;
        if self.stun_servers.iter().any(|url| {
            disabled.iter().any(|entry| {
                observer_address(entry).is_some()
                    && observer_address(entry) == observer_address(url)
            })
        }) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "RTC observer is disabled",
            ));
        }
        Ok(())
    }

    /// Existing native resource identities use authority URLs; WebRTC uses its
    /// standardized opaque STUN URI. Both refer to the same explicit observer.
    pub fn observation_resources(&self) -> io::Result<Vec<String>> {
        self.validate()?;
        Ok(self
            .stun_servers
            .iter()
            .map(|url| {
                format!(
                    "stun://{}/",
                    observer_address(url).expect("validated observer address")
                )
            })
            .collect())
    }

    fn candidate_allowed(&self, line: &str) -> io::Result<bool> {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if fields.len() < 8 || fields[6] != "typ" {
            return Err(invalid("RTC candidate is malformed"));
        }
        if fields[7] == "relay" {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "RTC direct profile rejects relay candidates",
            ));
        }
        if self.candidate_addresses.is_empty() {
            return Ok(true);
        }
        let address = fields[4].parse::<IpAddr>().ok();
        Ok(fields[1] == "1"
            && fields[2].eq_ignore_ascii_case("udp")
            && fields[5].parse::<u16>().is_ok_and(|port| port != 0)
            && address.is_some_and(|address| self.candidate_addresses.contains(&address)))
    }

    /// Keep the actual gathered SDP and remove only disallowed candidate lines.
    /// This is not a fabricated address or a separate signaling protocol.
    pub fn local_sdp(&self, sdp: &str) -> io::Result<String> {
        if sdp.len() > MAX_SDP_BYTES {
            return Err(invalid("RTC description exceeds its bound"));
        }
        self.validate()?;
        let mut published = String::with_capacity(sdp.len());
        for line in sdp.split_inclusive('\n') {
            if line.starts_with("a=candidate:") && !self.candidate_allowed(line)? {
                continue;
            }
            published.push_str(line);
        }
        Ok(published)
    }

    /// A peer cannot bypass this profile by supplying an unpublished host route.
    pub fn check_remote_sdp(&self, sdp: &str) -> io::Result<()> {
        if sdp.len() > MAX_SDP_BYTES {
            return Err(invalid("RTC description exceeds its bound"));
        }
        self.validate()?;
        for line in sdp.lines().filter(|line| line.starts_with("a=candidate:")) {
            if !self.candidate_allowed(line)? {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "RTC peer candidate is outside the explicit profile",
                ));
            }
        }
        Ok(())
    }
}
