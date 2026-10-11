//! Shared MIR2 wire contract. Routes and logical facilities confer no identity.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use iroh_base::{CustomAddr, RelayUrl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io};

pub const TRANSPORT_ID: u64 = 0x6d696f697263;
pub const MAX_DATAGRAM: usize = 2048;
pub const CHUNK: usize = 128;
pub const IRC_LINE_LIMIT: usize = 512;
/// Fixed MIR2 fields and separators, before the Base64 fragment.
const FRAGMENT_HEADER: usize = 94;
const MAX_FRAGMENT: usize = (IRC_LINE_LIMIT - 2 - FRAGMENT_HEADER) / 4 * 3;
const MAX_PARTS: usize = MAX_DATAGRAM.div_ceil(CHUNK);
pub const QUEUE_DEPTH: usize = 64;
pub type Route = [u8; 16];
pub type Service = [u8; 16];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    /// Fallback fragment interval for services without an active SAFERATE advertisement.
    pub pacing_ms: u64,
    pub assembly_ttl_ms: u64,
    pub queue_ttl_ms: u64,
    pub attempt_ms: u64,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            pacing_ms: 10,
            assembly_ttl_ms: 3000,
            queue_ttl_ms: 500,
            attempt_ms: 45000,
        }
    }
}
impl Profile {
    /// Owned slow-carrier experiment, never an implicit product deadline.
    pub fn slow() -> Self {
        Self {
            pacing_ms: 600,
            assembly_ttl_ms: 8000,
            queue_ttl_ms: 1500,
            attempt_ms: 30000,
        }
    }
    pub fn validate(&self) -> io::Result<()> {
        if !(1..=2000).contains(&self.pacing_ms)
            || !(100..=15000).contains(&self.assembly_ttl_ms)
            || !(1..=crate::connection_budget::MAX_MS).contains(&self.queue_ttl_ms)
            || !(100..=crate::connection_budget::MAX_MS).contains(&self.attempt_ms)
        {
            return Err(invalid("IRC profile exceeds bounded experiment limits"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IrcFacility {
    pub logical_id: String,
    pub native_url: String,
    pub wss_url: String,
    pub channel: String,
}
impl IrcFacility {
    pub fn validate(&self) -> io::Result<()> {
        if self.logical_id.is_empty()
            || self.logical_id.len() > 128
            || self.logical_id.chars().any(char::is_control)
            || !self.channel.starts_with('#')
            || !(2..=64).contains(&self.channel.len())
            || !self
                .channel
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"#-_".contains(&b))
        {
            return Err(invalid("invalid IRC facility or channel namespace"));
        }
        let native = entry(&self.native_url)?;
        let wss = entry(&self.wss_url)?;
        if !matches!(native.scheme(), "irc" | "ircs")
            || native.path() != "/"
            || wss.scheme() != "wss"
        {
            return Err(invalid("IRC facility requires native IRC and WSS entries"));
        }
        Ok(())
    }
    pub fn service_tag(&self) -> io::Result<Service> {
        self.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"miopunch/irc-facility/1\0");
        hash.update(self.logical_id.as_bytes());
        hash.update(b"\0");
        hash.update(self.channel.as_bytes());
        Ok(hash.finalize()[..16].try_into().unwrap())
    }
    pub fn check_allowed(&self, disabled: &[String]) -> io::Result<()> {
        self.validate()?;
        let native = origin(&self.native_url)?;
        let wss = origin(&self.wss_url)?;
        if disabled
            .iter()
            .any(|v| v == &self.logical_id || origin(v).is_ok_and(|v| v == native || v == wss))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "IRC facility disabled",
            ));
        }
        Ok(())
    }
}
fn entry(value: &str) -> io::Result<RelayUrl> {
    let url: RelayUrl = value.parse().map_err(|_| invalid("invalid IRC entry"))?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(invalid("invalid IRC entry options"));
    }
    Ok(url)
}
fn origin(value: &str) -> io::Result<String> {
    let u = entry(value)?;
    let p = u.port().unwrap_or(match u.scheme() {
        "irc" => 6667,
        "ircs" => 6697,
        "wss" => 443,
        _ => return Err(invalid("invalid IRC entry scheme")),
    });
    Ok(format!(
        "{}://{}:{p}",
        u.scheme(),
        u.host_str()
            .unwrap()
            .trim_end_matches('.')
            .to_ascii_lowercase()
    ))
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
pub fn address(service: Service, route: Route) -> CustomAddr {
    let mut data = [0; 32];
    data[..16].copy_from_slice(&service);
    data[16..].copy_from_slice(&route);
    CustomAddr::from_parts(TRANSPORT_ID, &data)
}
pub fn route_from_addr(addr: &CustomAddr, service: Service) -> io::Result<Route> {
    if addr.id() != TRANSPORT_ID || addr.data().len() != 32 || addr.data()[..16] != service {
        return Err(invalid("IRC route belongs to another service"));
    }
    Ok(addr.data()[16..].try_into().unwrap())
}
pub fn fragments(target: Route, source: Route, id: u64, bytes: &[u8]) -> io::Result<Vec<String>> {
    fragments_with_capacity(target, source, id, bytes, CHUNK)
}
pub fn fragments_with_capacity(
    target: Route,
    source: Route,
    id: u64,
    bytes: &[u8],
    capacity: usize,
) -> io::Result<Vec<String>> {
    if bytes.is_empty() || bytes.len() > MAX_DATAGRAM {
        return Err(invalid("invalid IRC datagram size"));
    }
    if capacity == 0 || capacity > MAX_FRAGMENT || bytes.len().div_ceil(capacity) > MAX_PARTS {
        return Err(invalid("IRC fragment capacity exceeds bounded wire limits"));
    }
    let total = bytes.len().div_ceil(capacity);
    Ok(bytes
        .chunks(capacity)
        .enumerate()
        .map(|(index, chunk)| {
            format!(
                "MIR2 {} {} {id:016x} {index:02x} {total:02x} {}",
                hex::encode(target),
                hex::encode(source),
                STANDARD.encode(chunk)
            )
        })
        .collect())
}
struct Assembly {
    created: u64,
    parts: Vec<Option<Vec<u8>>>,
    fragment_size: Option<usize>,
    bytes: usize,
}
#[derive(Default, Clone, Debug, Serialize)]
pub struct AssemblyReport {
    pub pending: usize,
    pub pending_peak: usize,
    pub expired: u64,
    pub rejected: u64,
    pub delivered: u64,
    pub completed_age_ms: u64,
}
pub struct Reassembly {
    cancelled: bool,
    profile: Profile,
    pending: BTreeMap<(Route, u64), Assembly>,
    retired: BTreeMap<(Route, u64), u64>,
    report: AssemblyReport,
}
impl Reassembly {
    pub fn new(profile: Profile) -> io::Result<Self> {
        profile.validate()?;
        Ok(Self {
            cancelled: false,
            profile,
            pending: BTreeMap::new(),
            retired: BTreeMap::new(),
            report: AssemblyReport::default(),
        })
    }
    fn retire(&mut self, key: (Route, u64), now: u64) {
        if self.retired.len() >= 128 {
            if let Some(key) = self
                .retired
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(key, _)| *key)
            {
                self.retired.remove(&key);
            }
        }
        self.retired.insert(key, now);
    }
    pub fn expire(&mut self, now: u64) {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, a)| now.saturating_sub(a.created) >= self.profile.assembly_ttl_ms)
            .map(|(key, _)| *key)
            .collect();
        for key in expired {
            self.pending.remove(&key);
            self.retire(key, now);
            self.report.expired += 1;
        }
        self.retired
            .retain(|_, at| now.saturating_sub(*at) < self.profile.assembly_ttl_ms);
        self.report.pending = self.pending.len();
    }
    pub fn snapshot(&self) -> AssemblyReport {
        self.report.clone()
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.pending.clear();
        self.retired.clear();
        self.report.pending = 0;
    }
    pub fn receive(&mut self, text: &str, own: Route, now: u64) -> Option<(Route, Vec<u8>)> {
        if self.cancelled {
            return None;
        }
        self.expire(now);
        if text.len() > FRAGMENT_HEADER + MAX_FRAGMENT.div_ceil(3) * 4 {
            self.report.rejected += 1;
            return None;
        }
        let f: Vec<_> = text.split(' ').collect();
        if f.len() != 7 || f[0] != "MIR2" || f[1] != hex::encode(own) {
            return None;
        }
        let parsed = (|| {
            let source: Route = hex::decode(f[2]).ok()?.try_into().ok()?;
            if source == own || f[3].len() != 16 || f[4].len() != 2 || f[5].len() != 2 {
                return None;
            }
            let id = u64::from_str_radix(f[3], 16).ok()?;
            let i = usize::from_str_radix(f[4], 16).ok()?;
            let n = usize::from_str_radix(f[5], 16).ok()?;
            if n == 0
                || n > MAX_PARTS
                || i >= n
                || f[6].is_empty()
                || f[6].len() > MAX_FRAGMENT.div_ceil(3) * 4
            {
                return None;
            }
            let bytes = STANDARD.decode(f[6]).ok()?;
            if bytes.is_empty() || bytes.len() > MAX_FRAGMENT {
                return None;
            }
            Some((source, id, i, n, bytes))
        })();
        let Some((source, id, i, n, bytes)) = parsed else {
            self.report.rejected += 1;
            return None;
        };
        let key = (source, id);
        if self.retired.contains_key(&key) {
            return None;
        }
        if !self.pending.contains_key(&key) && self.pending.len() >= 32 {
            self.report.rejected += 1;
            return None;
        }
        let assembly = self.pending.entry(key).or_insert_with(|| Assembly {
            created: now,
            parts: vec![None; n],
            fragment_size: None,
            bytes: 0,
        });
        let new_part = assembly.parts.get(i).is_some_and(Option::is_none);
        let fragment_size = if i + 1 < n {
            Some(bytes.len())
        } else {
            assembly.fragment_size
        };
        let inconsistent_size = assembly
            .fragment_size
            .zip(fragment_size)
            .is_some_and(|(old, new)| old != new)
            || fragment_size.is_some_and(|size| {
                (i + 1 == n && bytes.len() > size)
                    || assembly
                        .parts
                        .last()
                        .and_then(Option::as_ref)
                        .is_some_and(|last| last.len() > size)
                    || size * (n - 1) + 1 > MAX_DATAGRAM
            });
        if assembly.parts.len() != n
            || assembly.parts[i].as_ref().is_some_and(|old| old != &bytes)
            || inconsistent_size
            || assembly.bytes + if new_part { bytes.len() } else { 0 } > MAX_DATAGRAM
        {
            self.pending.remove(&key);
            self.retire(key, now);
            self.report.rejected += 1;
            self.report.pending = self.pending.len();
            return None;
        }
        if new_part {
            assembly.bytes += bytes.len();
            assembly.parts[i] = Some(bytes);
        }
        assembly.fragment_size = fragment_size;
        self.report.pending = self.pending.len();
        self.report.pending_peak = self.report.pending_peak.max(self.report.pending);
        if !self.pending[&key].parts.iter().all(Option::is_some) {
            return None;
        }
        let assembly = self.pending.remove(&key).unwrap();
        let bytes: Vec<_> = assembly.parts.into_iter().flatten().flatten().collect();
        self.retire(key, now);
        self.report.pending = self.pending.len();
        self.report.delivered += 1;
        self.report.completed_age_ms = now.saturating_sub(assembly.created);
        Some((source, bytes))
    }
}

pub struct Line<'a> {
    pub prefix: &'a str,
    pub command: &'a str,
    pub args: Vec<&'a str>,
    pub trailing: &'a str,
}
pub fn parse_line(line: &str) -> io::Result<Line<'_>> {
    if line.len() + 2 > IRC_LINE_LIMIT || line.contains(['\r', '\n', '\0']) {
        return Err(invalid("invalid IRC line"));
    }
    let (prefix, body) = if let Some(rest) = line.strip_prefix(':') {
        rest.split_once(' ').ok_or(invalid("invalid IRC prefix"))?
    } else {
        ("", line)
    };
    let (head, trailing) = body.split_once(" :").unwrap_or((body, ""));
    let mut words = head.split_whitespace();
    let command = words.next().ok_or(invalid("empty IRC line"))?;
    Ok(Line {
        prefix,
        command,
        args: words.collect(),
        trailing,
    })
}
#[derive(Default, Clone, Debug, Serialize)]
pub struct Qualification {
    retired: bool,
    pub registered: bool,
    pub capability: bool,
    pub joined: bool,
    pub names_done: bool,
    pub ready: bool,
    pub safe_rate: bool,
    own_prefix_bytes: Option<usize>,
    pub isupport: Vec<String>,
}
impl Qualification {
    pub fn receive(
        &mut self,
        line: &Line<'_>,
        nick: &str,
        channel: &str,
    ) -> io::Result<Vec<String>> {
        if self.retired {
            return Ok(Vec::new());
        }
        let mut control = vec![];
        match line.command {
            "PING" => {
                let token = if line.trailing.is_empty() {
                    line.args.first().copied().unwrap_or("")
                } else {
                    line.trailing
                };
                control.push(format!("PONG :{token}\r\n"));
            }
            "001" if line.args.first() == Some(&nick) => {
                if !self.registered {
                    self.registered = true;
                    control.push(format!("JOIN {channel}\r\n"));
                }
            }
            "005" if line.args.first() == Some(&nick) => {
                self.capability = true;
                for arg in line.args.iter().skip(1) {
                    match *arg {
                        "SAFERATE" => self.safe_rate = true,
                        "-SAFERATE" => self.safe_rate = false,
                        value if value.starts_with("SAFERATE=") => self.safe_rate = false,
                        _ => {}
                    }
                    if let Some(v) = arg.strip_prefix("NICKLEN=") {
                        if v.parse::<usize>().unwrap_or(0) < nick.len() {
                            return Err(invalid("IRC nick limit"));
                        }
                    }
                    if self.isupport.len() < 128 {
                        self.isupport.push((*arg).into());
                    }
                }
            }
            "JOIN"
                if line.prefix.split('!').next() == Some(nick)
                    && (line.args.first() == Some(&channel) || line.trailing == channel) =>
            {
                let (_, user_host) = line
                    .prefix
                    .split_once('!')
                    .ok_or(invalid("missing IRC user prefix"))?;
                let (user, host) = user_host
                    .split_once('@')
                    .ok_or(invalid("missing IRC host prefix"))?;
                if user.is_empty() || host.is_empty() {
                    return Err(invalid("invalid IRC user prefix"));
                }
                self.own_prefix_bytes = Some(line.prefix.len());
                self.fragment_capacity(channel)?;
                self.joined = true
            }
            "CHGHOST" if line.prefix.split('!').next() == Some(nick) && line.args.len() == 2 => {
                self.own_prefix_bytes =
                    Some(nick.len() + line.args[0].len() + line.args[1].len() + 2);
                self.fragment_capacity(channel)?;
            }
            "366" if line.args.first() == Some(&nick) && line.args.get(1) == Some(&channel) => {
                self.names_done = true
            }
            "KICK" if line.args.first() == Some(&channel) && line.args.get(1) == Some(&nick) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "IRC channel permission withdrawn",
                ));
            }
            "ERROR" | "403" | "404" | "405" | "407" | "421" | "433" | "439" | "471" | "473"
            | "474" | "475" | "477" | "479" => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("IRC service rejected: {}", line.command),
                ));
            }
            _ => {}
        }
        self.ready = self.registered && self.capability && self.joined && self.names_done;
        Ok(control)
    }
    /// A qualified service with SAFERATE handles rate limiting itself. Queue and
    /// socket bounds still apply; this never grants an unready or retired route.
    pub fn fragment_interval_ms(&self, profile: &Profile) -> u64 {
        if self.ready && self.safe_rate {
            0
        } else {
            profile.pacing_ms
        }
    }
    /// Budget the forwarded PRIVMSG, including `:nick!user@host ` and CRLF.
    pub fn fragment_capacity(&self, channel: &str) -> io::Result<usize> {
        let prefix = self
            .own_prefix_bytes
            .ok_or(invalid("IRC user prefix not qualified"))?;
        let remaining = IRC_LINE_LIMIT
            .checked_sub(prefix + channel.len() + 14 + FRAGMENT_HEADER)
            .ok_or(invalid("IRC prefix leaves no datagram capacity"))?;
        let capacity = remaining / 4 * 3;
        // Retain the existing sixteen-part and 2048-byte datagram bounds.
        if capacity < MAX_DATAGRAM.div_ceil(MAX_PARTS) {
            return Err(invalid("IRC prefix cannot carry bounded QUIC datagrams"));
        }
        Ok(capacity)
    }
    pub fn privmsg_line(&self, channel: &str, part: &str) -> io::Result<String> {
        if !self.ready || self.retired {
            return Err(invalid("IRC route not qualified"));
        }
        let prefix = self
            .own_prefix_bytes
            .ok_or(invalid("IRC user prefix not qualified"))?;
        let line = format!("PRIVMSG {channel} :{part}\r\n");
        if prefix + 2 + line.len() > IRC_LINE_LIMIT {
            return Err(invalid("IRC forwarded fragment exceeds line limit"));
        }
        Ok(line)
    }
    pub fn retire(&mut self) {
        self.retired = true;
        self.ready = false;
        self.safe_rate = false;
        self.own_prefix_bytes = None;
    }
}
