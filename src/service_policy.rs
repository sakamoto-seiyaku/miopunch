//! Signed node/service policies and temporary, single-identity invitations.
//! Native and WASM replay the same records against the existing management
//! graph. Platform adapters commit the complete verified record set before use.
use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p2panda_auth::Access;
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::management_auth::{ManagementAuth, NetworkAnchor};

pub const POLICY_ALPN: &[u8] = b"miopunch/service-policy/1";
pub const GUEST_ALPN: &[u8] = b"miopunch/guest-invitation/1";
pub const MAX_RECORDS: usize = 4096;

/// Private, short-lived bearer material. It contains neither a genesis seed
/// nor the network admission secret, and deliberately has no Debug output.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestInvitation {
    pub anchor: NetworkAnchor,
    pub issuer: iroh::EndpointAddr,
    pub provider: iroh::EndpointAddr,
    pub bootstrap: crate::client_invitation::BootstrapContact,
    pub record: Vec<u8>,
    pub token: [u8; 32],
}
impl GuestInvitation {
    pub fn id(&self) -> Hash {
        Hash::digest(&self.record)
    }
    pub fn details(&self) -> Result<GuestStatus, PolicyError> {
        self.validate()?;
        let header = Header::<Extension>::decode(&self.record)?;
        if let ServiceAction::InviteGuest {
            provider,
            service,
            created,
            expires,
            ..
        } = header.extensions.command
        {
            Ok(GuestStatus {
                invitation: self.id(),
                issuer: header.verifying_key,
                provider,
                service,
                created,
                expires,
                guest: None,
                ended: false,
            })
        } else {
            Err(PolicyError::Rejected)
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>, PolicyError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| PolicyError::Rejected)?;
        if bytes.len() > crate::client_invitation::MAX_INVITATION_BYTES {
            return Err(PolicyError::History);
        }
        Ok(bytes)
    }
    pub fn text(&self) -> Result<String, PolicyError> {
        Ok(format!(
            "miopunch://guest/{}",
            URL_SAFE_NO_PAD.encode(self.encode()?)
        ))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyError> {
        if bytes.len() > crate::client_invitation::MAX_INVITATION_BYTES * 2 {
            return Err(PolicyError::History);
        }
        let decoded;
        let bytes = if let Some(value) = std::str::from_utf8(bytes)
            .ok()
            .map(str::trim)
            .and_then(|s| s.strip_prefix("miopunch://guest/"))
        {
            decoded = URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| PolicyError::Rejected)?;
            decoded.as_slice()
        } else {
            bytes
        };
        if bytes.len() > crate::client_invitation::MAX_INVITATION_BYTES {
            return Err(PolicyError::History);
        }
        let invitation: Self = serde_json::from_slice(bytes).map_err(|_| PolicyError::Rejected)?;
        invitation.validate()?;
        Ok(invitation)
    }
    pub fn ordinary_contact(
        &self,
    ) -> Result<crate::client_invitation::ClientInvitation, PolicyError> {
        self.validate()?;
        let mut material = [0; 96];
        material[..32].copy_from_slice(self.anchor.root.as_bytes());
        material[32..64].copy_from_slice(self.anchor.genesis.as_bytes());
        material[64..].copy_from_slice(self.issuer.id.as_bytes());
        crate::client_invitation::ClientInvitation::new(
            material,
            self.issuer.clone(),
            None,
            self.bootstrap.clone(),
        )
        .map_err(|_| PolicyError::Rejected)
    }
    fn validate(&self) -> Result<(), PolicyError> {
        if self.record.len() > 16 * 1024
            || self.issuer.addrs.len() > 32
            || self.provider.addrs.len() > 32
        {
            return Err(PolicyError::History);
        }
        self.bootstrap
            .validate()
            .map_err(|_| PolicyError::Rejected)?;
        let header = Header::<Extension>::decode(&self.record)?;
        if header.extensions.anchor != self.anchor
            || header.extensions.version != 1
            || header.payload_size != 0
            || header.seq_num != 0
            || header.backlink.is_some()
            || header.verifying_key.as_bytes() != self.issuer.id.as_bytes()
        {
            return Err(PolicyError::Rejected);
        }
        match header.extensions.command {
            ServiceAction::InviteGuest {
                provider,
                service,
                token_hash,
                created,
                expires,
            } if provider.as_bytes() == self.provider.id.as_bytes()
                && service.guest_supported()
                && token_hash == Hash::digest(self.token)
                && created < expires =>
            {
                Ok(())
            }
            _ => Err(PolicyError::Rejected),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Files,
    Clipboard,
    TcpForward,
    UdpForward,
    Web,
    Terminal,
}
impl Service {
    /// One supported invitation boundary shared by encoders and platform APIs.
    pub fn guest_supported(self) -> bool {
        matches!(self, Self::Files | Self::Clipboard | Self::TcpForward | Self::UdpForward)
    }
}

/// One effective rule, edited through either authorized entry point. A guest
/// still needs its own live invitation; `guests` alone never grants access.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRule {
    pub members: bool,
    pub guests: bool,
    #[serde(default)]
    pub allowed: BTreeSet<VerifyingKey>,
    pub denied: BTreeSet<VerifyingKey>,
}
impl Default for ServiceRule {
    fn default() -> Self {
        Self {
            members: true,
            guests: true,
            allowed: BTreeSet::new(),
            denied: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceAction {
    Declare {
        provider: VerifyingKey,
        service: Service,
        previous: Option<Hash>,
        capable: bool,
        enabled: bool,
    },
    SetRule {
        provider: VerifyingKey,
        service: Service,
        previous: Option<Hash>,
        rule: ServiceRule,
    },
    InviteGuest {
        provider: VerifyingKey,
        service: Service,
        token_hash: Hash,
        created: u64,
        expires: u64,
    },
    BindGuest {
        invitation: Hash,
        guest: VerifyingKey,
    },
    EndGuest {
        invitation: Hash,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Extension {
    version: u8,
    anchor: NetworkAnchor,
    nonce: [u8; 16],
    management: Vec<Hash>,
    command: ServiceAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleStatus {
    pub provider: VerifyingKey,
    pub service: Service,
    pub revision: Option<Hash>,
    pub rule: ServiceRule,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceDeclaration {
    pub provider: VerifyingKey,
    pub service: Service,
    pub revision: Hash,
    pub capable: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestStatus {
    pub invitation: Hash,
    pub issuer: VerifyingKey,
    pub provider: VerifyingKey,
    pub service: Service,
    pub created: u64,
    pub expires: u64,
    pub guest: Option<VerifyingKey>,
    pub ended: bool,
}

#[derive(Clone)]
struct Guest {
    status: GuestStatus,
    token_hash: Hash,
    authority: Hash,
    binding: Option<Hash>,
    revision: Hash,
}

#[derive(Clone, Default)]
pub struct ServiceLedger {
    records: Vec<Vec<u8>>,
    ids: BTreeSet<Hash>,
    management: BTreeMap<Hash, Vec<Hash>>,
    rules: BTreeMap<(VerifyingKey, Service), (Hash, ServiceRule)>,
    declarations: BTreeMap<(VerifyingKey, Service), ServiceDeclaration>,
    guests: BTreeMap<Hash, Guest>,
}

#[derive(Clone)]
pub struct ServiceAccess {
    pub local: VerifyingKey,
    pub auth: ManagementAuth,
    pub ledger: ServiceLedger,
}
impl ServiceAccess {
    /// One signed provider rule, including a guest's exact invited service.
    pub fn permits_service(&self, service: Service, peer: VerifyingKey) -> bool {
        if self.member(self.local) {
            self.ledger
                .allows(self.local, service, peer, &self.auth, now())
        } else {
            self.ledger
                .guest_access(peer, service, self.local, &self.auth, now())
        }
    }
    pub fn declaration_change(
        &self,
        service: Service,
        capable: bool,
        enabled: bool,
    ) -> Option<ServiceAction> {
        if !self.member(self.local) {
            return None;
        }
        let previous = self.ledger.declaration(self.local, service);
        if previous.as_ref().is_some_and(|declaration| {
            declaration.capable == capable && declaration.enabled == enabled
        }) {
            return None;
        }
        Some(ServiceAction::Declare {
            provider: self.local,
            service,
            previous: previous.map(|declaration| declaration.revision),
            capable,
            enabled,
        })
    }
    pub fn local_service_status(
        &self,
        service: Service,
        capable: bool,
        enabled: bool,
    ) -> serde_json::Value {
        serde_json::json!({"service":service,"capable":capable,"enabled":enabled,
            "policy":self.ledger.rule(self.local, service)})
    }
    pub fn status(
        &self,
        invitation: Option<&GuestInvitation>,
        capable: bool,
        enabled: bool,
    ) -> Result<serde_json::Value, PolicyError> {
        let mut declarations = self.ledger.declarations();
        let mut policies = self.ledger.policies();
        let mut guests = self.ledger.guests();
        let service = invitation
            .map(GuestInvitation::details)
            .transpose()?
            .map_or(Service::Files, |scope| scope.service);
        let provider = if let Some(invitation) = invitation {
            let scope = invitation.details()?;
            let provider = scope.provider;
            declarations.retain(|d| d.provider == provider && d.service == scope.service);
            policies.retain(|p| p.provider == provider && p.service == scope.service);
            guests.retain(|g| g.invitation == invitation.id());
            provider
        } else {
            self.local
        };
        Ok(
            serde_json::json!({"node":self.local,"services":[{"service":service,"capable":capable,"enabled":enabled,
            "policy":self.ledger.rule(provider, service)}],"declarations":declarations,"policies":policies,"guests":guests}),
        )
    }
    pub fn guest_status(
        &self,
        invitation: &GuestInvitation,
    ) -> Result<serde_json::Value, PolicyError> {
        let status = self
            .ledger
            .guests()
            .into_iter()
            .find(|g| g.invitation == invitation.id())
            .unwrap_or(invitation.details()?);
        let active = !status.ended
            && now() < status.expires
            && status.guest == Some(self.local)
            && self.ledger.guest_access(
                status.provider,
                status.service,
                self.local,
                &self.auth,
                now(),
            );
        let stage = if active {
            "GuestActive"
        } else if status.ended {
            "GuestEnded"
        } else if now() >= status.expires {
            "GuestExpired"
        } else {
            "GuestPending"
        };
        Ok(serde_json::json!({"stage":stage,"role":"guest","scope":status}))
    }
    pub fn member(&self, peer: VerifyingKey) -> bool {
        self.auth.nodes().iter().any(|(id, _)| *id == peer)
    }
    pub fn can_receive(&self, peer: VerifyingKey) -> bool {
        self.permits_service(Service::Files, peer)
    }
    pub fn can_send(&self, peer: VerifyingKey) -> bool {
        if self.member(self.local) && self.member(peer) {
            self.ledger
                .allows(peer, Service::Files, self.local, &self.auth, now())
        } else {
            self.ledger
                .guest_access(peer, Service::Files, self.local, &self.auth, now())
                || self
                    .ledger
                    .guest_access(self.local, Service::Files, peer, &self.auth, now())
        }
    }
    pub fn guest_pairs(&self) -> Vec<([u8; 32], u64)> {
        self.guest_pairs_for(Service::Files)
    }
    pub fn guest_pairs_for(&self, service: Service) -> Vec<([u8; 32], u64)> {
        let current = now();
        let pairs = self
            .ledger
            .guests
            .values()
            .filter_map(|entry| {
                let guest = &entry.status;
                let id = guest.guest?;
                // The deadline must come from this exact live scoped record,
                // not another invitation which happens to authorize its peer.
                if guest.service != service
                    || self
                        .ledger
                        .rule(guest.provider, service)
                        .rule
                        .denied
                        .contains(&id)
                    || !self.ledger.invitation_live(entry, &self.auth, current)
                {
                    return None;
                }
                if self.local == guest.provider {
                    Some((*id.as_bytes(), guest.expires))
                } else if self.local == id {
                    Some((*guest.provider.as_bytes(), guest.expires))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let mut effective = BTreeMap::new();
        for (peer, expires) in pairs {
            effective
                .entry(peer)
                .and_modify(|deadline: &mut u64| *deadline = (*deadline).max(expires))
                .or_insert(expires);
        }
        effective.into_iter().collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("invalid signed service header: {0}")]
    Header(#[from] p2panda_core::HeaderError),
    #[error("invalid or unauthorized service record")]
    Rejected,
    #[error("service rule changed; refresh the effective revision before editing")]
    StaleRevision,
    #[error("service history is incomplete or exceeds its bound")]
    History,
}

impl ServiceLedger {
    pub fn declaration(
        &self,
        provider: VerifyingKey,
        service: Service,
    ) -> Option<ServiceDeclaration> {
        self.declarations.get(&(provider, service)).cloned()
    }
    pub fn declarations(&self) -> Vec<ServiceDeclaration> {
        self.declarations.values().cloned().collect()
    }
    pub fn policies(&self) -> Vec<RuleStatus> {
        let keys: BTreeSet<_> = self
            .declarations
            .keys()
            .chain(self.rules.keys())
            .copied()
            .collect();
        keys.into_iter()
            .map(|(provider, service)| self.rule(provider, service))
            .collect()
    }
    pub fn replay(
        anchor: NetworkAnchor,
        auth: &ManagementAuth,
        records: &[Vec<u8>],
    ) -> Result<Self, PolicyError> {
        let mut ledger = Self::default();
        for record in records {
            ledger.accept(anchor, auth, record)?;
        }
        Ok(ledger)
    }
    pub fn records(&self) -> &[Vec<u8>] {
        &self.records
    }
    pub fn rule(&self, provider: VerifyingKey, service: Service) -> RuleStatus {
        let (revision, rule) = self
            .rules
            .get(&(provider, service))
            .map(|(id, rule)| (Some(*id), rule.clone()))
            .unwrap_or_default();
        RuleStatus {
            provider,
            service,
            revision,
            rule,
        }
    }
    pub fn guests(&self) -> Vec<GuestStatus> {
        self.guests
            .values()
            .map(|guest| guest.status.clone())
            .collect()
    }
    pub fn is_guest_end(bytes: &[u8], peer: VerifyingKey) -> bool {
        Header::<Extension>::decode(bytes).is_ok_and(|header| {
            header.verifying_key == peer
                && matches!(header.extensions.command, ServiceAction::EndGuest { .. })
        })
    }
    pub fn guest_end_record(&self, invitation: Hash) -> Option<Vec<u8>> {
        self.records.iter().rev().find(|bytes| Header::<Extension>::decode(bytes).is_ok_and(|header|
            matches!(header.extensions.command, ServiceAction::EndGuest { invitation: id } if id == invitation))).cloned()
    }
    pub fn author(
        &self,
        anchor: NetworkAnchor,
        auth: &ManagementAuth,
        key: &SigningKey,
        command: ServiceAction,
    ) -> Result<Vec<u8>, PolicyError> {
        let extension = Extension {
            version: 1,
            anchor,
            nonce: rand::random(),
            management: auth.tips(),
            command,
        };
        let bytes = Header::<Extension>::builder()
            .build(key, extension)
            .encode();
        self.merge(anchor, auth, std::slice::from_ref(&bytes))?;
        Ok(bytes)
    }
    pub fn merge(
        &self,
        anchor: NetworkAnchor,
        auth: &ManagementAuth,
        records: &[Vec<u8>],
    ) -> Result<Self, PolicyError> {
        let mut next = self.clone();
        for record in records {
            next.accept(anchor, auth, record)?;
        }
        // Replay preserves accepted history. A newly arriving effective head
        // must also have a still-valid authorization episode now. Intermediate
        // history can be carried by a current, authorized successor.
        for id in next
            .rules
            .values()
            .map(|(id, _)| *id)
            .chain(next.declarations.values().map(|d| d.revision))
            .chain(next.guests.values().map(|g| g.revision))
            .filter(|id| !self.ids.contains(id))
        {
            let record = next
                .records
                .iter()
                .find(|bytes| Hash::digest(bytes) == id)
                .ok_or(PolicyError::History)?;
            next.validate_current_authority(auth, record)?;
        }
        Ok(next)
    }

    fn validate_current_authority(
        &self,
        auth: &ManagementAuth,
        bytes: &[u8],
    ) -> Result<(), PolicyError> {
        let header = Header::<Extension>::decode(bytes)?;
        let e = &header.extensions;
        let actor = header.verifying_key;
        let provider = match &e.command {
            ServiceAction::Declare { provider, .. }
            | ServiceAction::SetRule { provider, .. }
            | ServiceAction::InviteGuest { provider, .. } => *provider,
            ServiceAction::BindGuest { invitation, .. }
            | ServiceAction::EndGuest { invitation } => {
                let guest = self.guests.get(invitation).ok_or(PolicyError::History)?;
                if matches!(e.command, ServiceAction::EndGuest { .. })
                    && guest.status.guest == Some(actor)
                {
                    return Ok(());
                }
                guest.status.provider
            }
        };
        if !auth.nodes().iter().any(|(id, _)| *id == provider) {
            return Err(PolicyError::Rejected);
        }
        let administrator = actor != provider;
        let origin = auth
            .authorization_origin_at(actor, &e.management, administrator)
            .ok_or(PolicyError::Rejected)?;
        let valid = if administrator {
            auth.administrator_authorization_valid(origin, actor)
        } else {
            auth.membership_authorization_valid(origin, actor)
        };
        if !valid {
            return Err(PolicyError::Rejected);
        }
        Ok(())
    }
    fn accept(
        &mut self,
        anchor: NetworkAnchor,
        auth: &ManagementAuth,
        bytes: &[u8],
    ) -> Result<(), PolicyError> {
        if bytes.len() > 16 * 1024 {
            return Err(PolicyError::History);
        }
        let header = Header::<Extension>::decode(bytes)?;
        let id = header.hash();
        if self.ids.contains(&id) {
            return Ok(());
        }
        if self.records.len() >= MAX_RECORDS {
            return Err(PolicyError::History);
        }
        let e = &header.extensions;
        if e.version != 1
            || e.anchor != anchor
            || header.payload_size != 0
            || header.seq_num != 0
            || header.backlink.is_some()
        {
            return Err(PolicyError::Rejected);
        }
        let nodes = auth
            .nodes_at(&e.management)
            .map_err(|_| PolicyError::History)?;
        let predecessor = match &e.command {
            ServiceAction::Declare { previous, .. } | ServiceAction::SetRule { previous, .. } => {
                *previous
            }
            ServiceAction::BindGuest { invitation, .. } => Some(*invitation),
            ServiceAction::EndGuest { invitation } => Some(
                self.guests
                    .get(invitation)
                    .ok_or(PolicyError::History)?
                    .revision,
            ),
            ServiceAction::InviteGuest { .. } => None,
        };
        if let Some(previous) = predecessor {
            let required = self.management.get(&previous).ok_or(PolicyError::History)?;
            if !auth.covers(&e.management, required) {
                return Err(PolicyError::Rejected);
            }
        }
        let member = |node| nodes.iter().any(|(id, _)| *id == node);
        let manager = |provider| {
            member(provider)
                && (header.verifying_key == provider
                    || nodes.contains(&(header.verifying_key, Access::manage())))
        };
        match &e.command {
            ServiceAction::Declare {
                provider,
                service,
                previous,
                capable,
                enabled,
            } => {
                if header.verifying_key != *provider || !member(*provider) || *enabled && !*capable
                {
                    return Err(PolicyError::Rejected);
                }
                if self.declaration(*provider, *service).map(|d| d.revision) != *previous {
                    return Err(PolicyError::StaleRevision);
                }
                self.declarations.insert(
                    (*provider, *service),
                    ServiceDeclaration {
                        provider: *provider,
                        service: *service,
                        revision: id,
                        capable: *capable,
                        enabled: *enabled,
                    },
                );
            }
            ServiceAction::SetRule {
                provider,
                service,
                previous,
                rule,
            } => {
                if !manager(*provider) || rule.denied.len() + rule.allowed.len() > 256 {
                    return Err(PolicyError::Rejected);
                }
                if self.rule(*provider, *service).revision != *previous {
                    return Err(PolicyError::StaleRevision);
                }
                self.rules.insert((*provider, *service), (id, rule.clone()));
            }
            ServiceAction::InviteGuest {
                provider,
                service,
                token_hash,
                created,
                expires,
            } => {
                if !manager(*provider) || *expires <= *created || *expires > i64::MAX as u64 {
                    return Err(PolicyError::Rejected);
                }
                self.guests.insert(
                    id,
                    Guest {
                        status: GuestStatus {
                            invitation: id,
                            issuer: header.verifying_key,
                            provider: *provider,
                            service: *service,
                            created: *created,
                            expires: *expires,
                            guest: None,
                            ended: false,
                        },
                        token_hash: *token_hash,
                        authority: auth
                            .authorization_origin_at(
                                header.verifying_key,
                                &e.management,
                                header.verifying_key != *provider,
                            )
                            .ok_or(PolicyError::Rejected)?,
                        binding: None,
                        revision: id,
                    },
                );
            }
            ServiceAction::BindGuest { invitation, guest } => {
                let entry = self
                    .guests
                    .get_mut(invitation)
                    .ok_or(PolicyError::History)?;
                if header.verifying_key != entry.status.issuer
                    || !manager(entry.status.provider)
                    || entry.status.ended
                    || entry.status.guest.is_some()
                    || member(*guest)
                {
                    return Err(PolicyError::Rejected);
                }
                entry.status.guest = Some(*guest);
                entry.binding = Some(id);
                entry.revision = id;
            }
            ServiceAction::EndGuest { invitation } => {
                let entry = self
                    .guests
                    .get_mut(invitation)
                    .ok_or(PolicyError::History)?;
                if Some(header.verifying_key) != entry.status.guest
                    && !manager(entry.status.provider)
                {
                    return Err(PolicyError::Rejected);
                }
                entry.status.ended = true;
                entry.revision = id;
            }
        }
        self.ids.insert(id);
        self.management.insert(id, e.management.clone());
        self.records.push(bytes.to_vec());
        Ok(())
    }
    pub fn redeem(
        &self,
        invitation: Hash,
        token: &[u8; 32],
        guest: VerifyingKey,
        auth: &ManagementAuth,
        now: u64,
    ) -> Result<bool, PolicyError> {
        let entry = self.guests.get(&invitation).ok_or(PolicyError::Rejected)?;
        if entry.token_hash != Hash::digest(token)
            || !self.invitation_live(entry, auth, now)
            || entry.status.guest.is_some_and(|id| id != guest)
            || auth.nodes().iter().any(|(id, _)| *id == guest)
        {
            return Err(PolicyError::Rejected);
        }
        Ok(entry.status.guest.is_none())
    }
    fn invitation_live(&self, entry: &Guest, auth: &ManagementAuth, now: u64) -> bool {
        let s = &entry.status;
        let nodes = auth.nodes();
        // Losing the issuing role or membership invalidates its old invitations.
        let issuer_live = if s.issuer == s.provider {
            auth.membership_authorization_valid(entry.authority, s.issuer)
        } else {
            auth.administrator_authorization_valid(entry.authority, s.issuer)
        };
        !s.ended
            && now < s.expires
            && issuer_live
            && nodes.iter().any(|(id, _)| *id == s.provider)
            && self.rule(s.provider, s.service).rule.guests
    }
    pub fn guest_access(
        &self,
        provider: VerifyingKey,
        service: Service,
        guest: VerifyingKey,
        auth: &ManagementAuth,
        now: u64,
    ) -> bool {
        let rule = self.rule(provider, service).rule;
        !rule.denied.contains(&guest)
            && self.guests.values().any(|entry| {
                entry.status.provider == provider
                    && entry.status.service == service
                    && entry.status.guest == Some(guest)
                    && self.invitation_live(entry, auth, now)
            })
    }
    pub fn allows(
        &self,
        provider: VerifyingKey,
        service: Service,
        peer: VerifyingKey,
        auth: &ManagementAuth,
        now: u64,
    ) -> bool {
        let rule = self.rule(provider, service).rule;
        auth.nodes().iter().any(|(id, _)| *id == provider)
            && !rule.denied.contains(&peer)
            && ((rule.members || rule.allowed.contains(&peer))
                && auth.nodes().iter().any(|(id, _)| *id == peer)
                || self.guest_access(provider, service, peer, auth, now))
    }
}

pub fn now() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}
