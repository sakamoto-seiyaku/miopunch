//! Bounded transfer of original signed control records. No admission secret,
//! invitation bearer or business file content is included in this history.
use crate::{
    data_stream::DataStream,
    device_name::DeviceNames,
    management_auth::{ManagementAuth, NetworkAnchor},
    service_policy::{MAX_RECORDS, ServiceLedger},
};
use p2panda_core::{Hash, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::io;

pub type ManagementRecords = Vec<(Vec<u8>, Option<Vec<u8>>)>;
#[derive(Serialize, Deserialize)]
pub enum GuestRequest {
    Redeem {
        invitation: p2panda_core::Hash,
        token: [u8; 32],
    },
    End {
        record: Vec<u8>,
    },
}
pub async fn write_guest(
    exchange: &iroh::endpoint::ControlExchange,
    value: &impl Serialize,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    exchange
        .send_message(
            iroh::endpoint::ControlMessageFormat::LengthPrefixed32 {
                maximum: 16 * 1024 * 1024,
            },
            &bytes,
        )
        .await
        .map_err(io::Error::other)
}
pub async fn read_guest<T: serde::de::DeserializeOwned>(
    exchange: &iroh::endpoint::ControlExchange,
) -> io::Result<T> {
    let bytes = exchange
        .recv_message(iroh::endpoint::ControlMessageFormat::LengthPrefixed32 {
            maximum: 16 * 1024 * 1024,
        })
        .await
        .map_err(io::Error::other)?;
    Ok(serde_json::from_slice(&bytes)?)
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ServiceBundle {
    pub management: ManagementRecords,
    pub services: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum MemberSyncMode {
    Full,
    Management,
}

#[derive(Clone)]
pub(crate) struct MemberSyncMembership {
    local: (VerifyingKey, Hash),
    peer: (VerifyingKey, Hash),
}

impl MemberSyncMembership {
    pub(crate) fn capture(
        auth: &ManagementAuth,
        local: VerifyingKey,
        peer: iroh_base::EndpointId,
    ) -> Option<Self> {
        let peer = VerifyingKey::from_bytes(peer.as_bytes()).ok()?;
        let tips = auth.tips();
        Some(Self {
            local: (local, auth.authorization_origin_at(local, &tips, false)?),
            peer: (peer, auth.authorization_origin_at(peer, &tips, false)?),
        })
    }

    pub(crate) fn is_current(&self, auth: &ManagementAuth, local: VerifyingKey) -> bool {
        self.local.0 == local
            && [self.local, self.peer]
                .into_iter()
                .all(|(id, grant)| auth.membership_authorization_valid(grant, id))
    }
}

/// The admitted member exchange carries management history in either mode.
/// Explicit service synchronization also shares service policies and names.
#[derive(Serialize, Deserialize)]
pub(crate) struct MemberSync {
    #[serde(flatten)]
    pub(crate) bundle: ServiceBundle,
    #[serde(default, skip_serializing_if = "DeviceNames::is_empty")]
    pub(crate) device_names: DeviceNames,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    management_only: bool,
}

// A distinct outer shape makes old receivers reject a paged transfer before
// they can mistake its first page for a complete history.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum MemberSyncWire {
    Offer {
        member_sync_offer: Vec<Hash>,
        management_only: bool,
    },
    Page {
        member_sync_page: MemberSync,
        continues: bool,
    },
    Legacy(MemberSync),
}

#[derive(Serialize, Deserialize)]
struct MissingManagement {
    missing: Vec<Hash>,
}

#[derive(Serialize, Deserialize)]
struct SavedManagement {
    saved: bool,
}

impl MemberSync {
    pub(crate) fn new(
        bundle: ServiceBundle,
        device_names: DeviceNames,
        mode: MemberSyncMode,
    ) -> Self {
        let mut sync = Self {
            bundle,
            device_names,
            management_only: false,
        };
        sync.restrict(mode);
        sync
    }

    pub(crate) fn mode(&self) -> MemberSyncMode {
        if self.management_only {
            MemberSyncMode::Management
        } else {
            MemberSyncMode::Full
        }
    }

    fn restrict(&mut self, mode: MemberSyncMode) {
        if mode == MemberSyncMode::Management {
            self.management_only = true;
            self.bundle.services.clear();
            self.device_names = DeviceNames::default();
        }
    }

    pub(crate) async fn exchange(
        &self,
        connection: &iroh::endpoint::Connection,
        anchor: NetworkAnchor,
    ) -> io::Result<Self> {
        let mut stream = DataStream::open(connection).await?;
        self.write(&mut stream).await?;
        let mut reply = Self::read(&mut stream, anchor).await?;
        // Older responders return the full bundle regardless of the flag.
        reply.restrict(self.mode());
        Ok(reply)
    }

    pub(crate) async fn write(&self, stream: &mut DataStream) -> io::Result<()> {
        if self.bundle.management.len() <= MAX_RECORDS
            && self
                .bundle
                .management
                .iter()
                .map(|(header, body)| header.len() + body.as_ref().map_or(0, Vec::len))
                .sum::<usize>()
                <= MAX_BUNDLE_BYTES
        {
            let bytes = serde_json::to_vec(self)?;
            if bytes.len() <= MAX_BUNDLE_BYTES {
                return write_bundle_bytes(&bytes, stream).await;
            }
        }
        // Validate the final policies and names before emitting any history.
        let final_page = MemberSync {
            bundle: ServiceBundle {
                management: Vec::new(),
                services: self.bundle.services.clone(),
            },
            device_names: self.device_names.clone(),
            management_only: self.management_only,
        };
        let final_bytes = serde_json::to_vec(&MemberSyncWire::Page {
            member_sync_page: final_page,
            continues: false,
        })?;
        if final_bytes.len() > MAX_BUNDLE_BYTES {
            return Err(rejected());
        }
        let mut remaining = self.bundle.management.as_slice();
        while !remaining.is_empty() {
            let (count, bytes) =
                crate::management_record::encode_history_page(remaining, |records| {
                    serde_json::to_vec(&MemberSyncWire::Page {
                        member_sync_page: MemberSync {
                            bundle: ServiceBundle {
                                management: records.to_vec(),
                                services: Vec::new(),
                            },
                            device_names: DeviceNames::default(),
                            management_only: self.management_only,
                        },
                        continues: true,
                    })
                    .map_err(io::Error::other)
                })?;
            write_bundle_bytes(&bytes, stream).await?;
            remaining = &remaining[count..];
        }
        write_bundle_bytes(&final_bytes, stream).await
    }

    pub(crate) async fn write_resuming(&self, stream: &mut DataStream) -> io::Result<()> {
        const TARGET_RECORDS: usize = 128;
        const TARGET_BYTES: usize = 1024 * 1024;
        if self.bundle.management.len() <= TARGET_RECORDS
            && self
                .bundle
                .management
                .iter()
                .map(|(header, body)| header.len() + body.as_ref().map_or(0, Vec::len))
                .sum::<usize>()
                <= TARGET_BYTES
        {
            let bytes = serde_json::to_vec(self)?;
            if bytes.len() <= TARGET_BYTES {
                return write_bundle_bytes(&bytes, stream).await;
            }
        }
        let mut remaining = self.bundle.management.as_slice();
        while !remaining.is_empty() {
            let mut limit = remaining.len().min(TARGET_RECORDS);
            let count = loop {
                let (count, bytes) =
                    crate::management_record::raw_records_page(&remaining[..limit], 0)?;
                if bytes.len() <= TARGET_BYTES || count == 1 {
                    break count;
                }
                limit = count.div_ceil(2);
            };
            let page = &remaining[..count];
            let ids: Vec<_> = page
                .iter()
                .map(|(header, _)| Hash::digest(header))
                .collect();
            write_bundle_bytes(
                &serde_json::to_vec(&MemberSyncWire::Offer {
                    member_sync_offer: ids.clone(),
                    management_only: self.management_only,
                })?,
                stream,
            )
            .await?;
            let needed: MissingManagement = read_bundle(stream).await?;
            let missing: ManagementRecords = page
                .iter()
                .zip(&ids)
                .filter(|(_, id)| needed.missing.contains(id))
                .map(|(record, _)| record.clone())
                .collect();
            if needed.missing
                != missing
                    .iter()
                    .map(|(header, _)| Hash::digest(header))
                    .collect::<Vec<_>>()
            {
                return Err(rejected());
            }
            write_bundle_bytes(
                &crate::management_record::encode_raw_records(&missing)?,
                stream,
            )
            .await?;
            let ack: SavedManagement = read_bundle(stream).await?;
            if !ack.saved {
                return Err(rejected());
            }
            remaining = &remaining[count..];
        }
        let final_page = MemberSyncWire::Page {
            member_sync_page: Self {
                bundle: ServiceBundle {
                    management: Vec::new(),
                    services: self.bundle.services.clone(),
                },
                device_names: self.device_names.clone(),
                management_only: self.management_only,
            },
            continues: false,
        };
        write_bundle_bytes(&serde_json::to_vec(&final_page)?, stream).await
    }

    pub(crate) async fn exchange_resuming<F, Fut>(
        &self,
        connection: &iroh::endpoint::Connection,
        anchor: NetworkAnchor,
        known: ManagementAuth,
        persist: F,
    ) -> io::Result<Self>
    where
        F: FnMut(ManagementRecords) -> Fut,
        Fut: std::future::Future<Output = io::Result<()>>,
    {
        let mut stream = DataStream::open(connection).await?;
        self.write_resuming(&mut stream).await?;
        let mut reply = Self::read_resuming(
            &mut stream,
            anchor,
            self.bundle.management.clone(),
            known,
            persist,
        )
        .await?;
        reply.restrict(self.mode());
        Ok(reply)
    }

    pub(crate) async fn read(stream: &mut DataStream, anchor: NetworkAnchor) -> io::Result<Self> {
        Self::read_with_anchor(stream, Some(anchor)).await
    }

    async fn read_with_anchor(
        stream: &mut DataStream,
        anchor: Option<NetworkAnchor>,
    ) -> io::Result<Self> {
        Self::read_resuming_inner(stream, anchor, Vec::new(), None, |_| async { Ok(()) }).await
    }

    /// The records and authority view come from one verified local snapshot.
    pub(crate) async fn read_resuming<F, Fut>(
        stream: &mut DataStream,
        anchor: NetworkAnchor,
        local: ManagementRecords,
        known: ManagementAuth,
        persist: F,
    ) -> io::Result<Self>
    where
        F: FnMut(ManagementRecords) -> Fut,
        Fut: std::future::Future<Output = io::Result<()>>,
    {
        Self::read_resuming_inner(stream, Some(anchor), local, Some(known), persist).await
    }

    async fn read_resuming_inner<F, Fut>(
        stream: &mut DataStream,
        anchor: Option<NetworkAnchor>,
        local: ManagementRecords,
        known: Option<ManagementAuth>,
        mut persist: F,
    ) -> io::Result<Self>
    where
        F: FnMut(ManagementRecords) -> Fut,
        Fut: std::future::Future<Output = io::Result<()>>,
    {
        let mut auth = known.or_else(|| anchor.map(ManagementAuth::new));
        let mut management = local;
        let mut wire = read_bundle(stream).await?;
        let mut resume_mode = None;
        loop {
            let MemberSyncWire::Offer {
                member_sync_offer: ids,
                management_only,
            } = wire
            else {
                break;
            };
            let verified = auth.as_mut().ok_or_else(rejected)?;
            if ids.is_empty()
                || ids.len() > MAX_RECORDS
                || ids
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != ids.len()
                || resume_mode.is_some_and(|mode| mode != management_only)
            {
                return Err(rejected());
            }
            resume_mode = Some(management_only);
            let missing: Vec<_> = ids
                .into_iter()
                .filter(|id| !verified.contains_operation(*id))
                .collect();
            write_bundle_bytes(
                &serde_json::to_vec(&MissingManagement {
                    missing: missing.clone(),
                })?,
                stream,
            )
            .await?;
            let bytes = read_bundle_bytes(stream).await?;
            let page = crate::management_record::decode_raw_records(&bytes)?;
            if page.len() != missing.len() {
                return Err(rejected());
            }
            for ((header, body), expected) in page.iter().zip(&missing) {
                if Hash::digest(header) != *expected {
                    return Err(rejected());
                }
                verified
                    .accept_record(header, body.as_deref())
                    .map_err(|_| rejected())?;
            }
            if !page.is_empty() {
                persist(page.clone()).await?;
            }
            management.extend(page);
            write_bundle_bytes(
                &serde_json::to_vec(&SavedManagement { saved: true })?,
                stream,
            )
            .await?;
            wire = read_bundle(stream).await?;
        }
        let (mut sync, mut continues) = match wire {
            MemberSyncWire::Legacy(sync) if resume_mode.is_none() => (sync, false),
            MemberSyncWire::Legacy(_) => return Err(rejected()),
            MemberSyncWire::Page {
                member_sync_page,
                continues,
            } if anchor.is_some() => (member_sync_page, continues),
            MemberSyncWire::Page { .. } | MemberSyncWire::Offer { .. } => return Err(rejected()),
        };
        if sync.bundle.management.len() > MAX_RECORDS {
            return Err(rejected());
        }
        if resume_mode.is_some_and(|mode| {
            mode != sync.management_only || continues || !sync.bundle.management.is_empty()
        }) {
            return Err(rejected());
        }
        loop {
            if continues && sync.bundle.management.is_empty() {
                return Err(rejected());
            }
            if let Some(auth) = &mut auth {
                sync.bundle
                    .management
                    .retain(|(header, _)| !auth.contains_operation(Hash::digest(header)));
                for (header, body) in &sync.bundle.management {
                    auth.accept_record(header, body.as_deref())
                        .map_err(|_| rejected())?;
                }
            }
            if !continues {
                break;
            }
            if !sync.bundle.services.is_empty() || !sync.device_names.is_empty() {
                return Err(rejected());
            }
            management.append(&mut sync.bundle.management);
            let MemberSyncWire::Page {
                member_sync_page: next,
                continues: more,
            } = read_bundle(stream).await?
            else {
                return Err(rejected());
            };
            if next.mode() != sync.mode() || next.bundle.management.len() > MAX_RECORDS {
                return Err(rejected());
            }
            sync = next;
            continues = more;
        }
        management.append(&mut sync.bundle.management);
        sync.bundle.management = management;
        sync.restrict(sync.mode());
        Ok(sync)
    }
}

pub(crate) fn next_member_sync_peer(
    peers: impl IntoIterator<Item = iroh_base::EndpointId>,
    after: Option<iroh_base::EndpointId>,
) -> Option<iroh_base::EndpointId> {
    peers
        .into_iter()
        .min_by_key(|peer| (after.is_some_and(|id| *peer <= id), *peer))
}
/// Verified management is publishable even when accompanying service changes
/// are rejected. Adapters commit this bundle before returning policy_error.
pub struct ServiceMerge {
    pub bundle: ServiceBundle,
    pub policy_error: Option<io::Error>,
}
impl ServiceBundle {
    pub fn merged(&self, anchor: NetworkAnchor, incoming: &Self) -> io::Result<Self> {
        let result = self.reconcile(anchor, incoming)?;
        if let Some(error) = result.policy_error {
            return Err(error);
        }
        Ok(result.bundle)
    }
    pub fn reconcile(&self, anchor: NetworkAnchor, incoming: &Self) -> io::Result<ServiceMerge> {
        ManagementAuth::replay_records(anchor, &incoming.management).map_err(|_| rejected())?;
        let mut management = self.management.clone();
        let existing: std::collections::BTreeSet<_> = management
            .iter()
            .map(|(header, _)| p2panda_core::Hash::digest(header))
            .collect();
        management.extend(
            incoming
                .management
                .iter()
                .filter(|(header, _)| !existing.contains(&p2panda_core::Hash::digest(header)))
                .cloned(),
        );
        let auth = ManagementAuth::replay_records(anchor, &management).map_err(|_| rejected())?;
        let current =
            ServiceLedger::replay(anchor, &auth, &self.services).map_err(|_| rejected())?;
        let policies = if incoming.services.len() > MAX_RECORDS {
            Err(rejected())
        } else {
            current
                .merge(anchor, &auth, &incoming.services)
                .map_err(|_| rejected())
        };
        let (ledger, policy_error) = match policies {
            Ok(ledger) => (ledger, None),
            Err(error) => (current, Some(error)),
        };
        Ok(ServiceMerge {
            bundle: Self {
                management,
                services: ledger.records().to_vec(),
            },
            policy_error,
        })
    }
    pub fn verify(&self, anchor: NetworkAnchor) -> io::Result<(ManagementAuth, ServiceLedger)> {
        if self.services.len() > MAX_RECORDS {
            return Err(rejected());
        }
        let auth =
            ManagementAuth::replay_records(anchor, &self.management).map_err(|_| rejected())?;
        let ledger =
            ServiceLedger::replay(anchor, &auth, &self.services).map_err(|_| rejected())?;
        Ok((auth, ledger))
    }
    pub async fn write(&self, stream: &mut DataStream) -> io::Result<()> {
        MemberSync::new(self.clone(), DeviceNames::default(), MemberSyncMode::Full)
            .write(stream)
            .await
    }
    /// Read a bounded legacy bundle. Paged histories use read_verified.
    pub async fn read(stream: &mut DataStream) -> io::Result<Self> {
        Ok(MemberSync::read_with_anchor(stream, None).await?.bundle)
    }
    /// Read paged management history against this node's trusted network.
    pub async fn read_verified(stream: &mut DataStream, anchor: NetworkAnchor) -> io::Result<Self> {
        Ok(MemberSync::read(stream, anchor).await?.bundle)
    }
}

const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;

async fn write_bundle_bytes(bytes: &[u8], stream: &mut DataStream) -> io::Result<()> {
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(rejected());
    }
    for chunk in bytes.chunks(crate::data_stream::CHUNK_BYTES) {
        stream.write_chunk(chunk).await?;
    }
    stream.write_chunk(&[]).await
}

async fn read_bundle<T: serde::de::DeserializeOwned>(stream: &mut DataStream) -> io::Result<T> {
    let bytes = read_bundle_bytes(stream).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn read_bundle_bytes(stream: &mut DataStream) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = stream.read_chunk().await?;
        if chunk.is_empty() {
            break;
        }
        if bytes.len() + chunk.len() > MAX_BUNDLE_BYTES {
            return Err(rejected());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn rejected() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "invalid service history")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn transfer(sync: &MemberSync, anchor: NetworkAnchor) -> io::Result<MemberSync> {
        let (mut sending, mut wire, _unused) = DataStream::local(128 * 1024);
        let (mut receiving, _unused, mut input) = DataStream::local(128 * 1024);
        let writer = async {
            sync.write(&mut sending).await?;
            sending.finish()
        };
        let relay = async {
            tokio::io::copy(&mut wire, &mut input).await?;
            input.shutdown().await
        };
        let (_, _, received) =
            tokio::try_join!(writer, relay, MemberSync::read(&mut receiving, anchor))?;
        Ok(received)
    }

    async fn transfer_resuming<F, Fut>(
        sync: &MemberSync,
        anchor: NetworkAnchor,
        local: ManagementRecords,
        persist: F,
    ) -> io::Result<MemberSync>
    where
        F: FnMut(ManagementRecords) -> Fut,
        Fut: std::future::Future<Output = io::Result<()>>,
    {
        let (mut sending, mut outgoing, mut sender_input) = DataStream::local(128 * 1024);
        let (mut receiving, mut feedback, mut receiver_input) = DataStream::local(128 * 1024);
        let sender = async {
            sync.write_resuming(&mut sending).await?;
            sending.finish()
        };
        let up = async {
            tokio::io::copy(&mut outgoing, &mut receiver_input).await?;
            receiver_input.shutdown().await
        };
        let down = async {
            tokio::io::copy(&mut feedback, &mut sender_input).await?;
            sender_input.shutdown().await
        };
        let receiver = async {
            let result = MemberSync::read_resuming(
                &mut receiving,
                anchor,
                local.clone(),
                ManagementAuth::replay_records(anchor, &local).unwrap(),
                persist,
            )
            .await?;
            receiving.finish()?;
            Ok::<_, io::Error>(result)
        };
        let (_, _, _, result) = tokio::try_join!(sender, up, down, receiver)?;
        Ok(result)
    }

    #[tokio::test]
    async fn member_sync_pages_record_count_and_rejects_invalid_pages() {
        let records: ManagementRecords = (0..MAX_RECORDS + 3)
            .map(|n| (n.to_be_bytes().to_vec(), None))
            .collect();
        let sync = MemberSync::new(
            ServiceBundle {
                management: records.clone(),
                services: vec![vec![3]],
            },
            DeviceNames::default(),
            MemberSyncMode::Full,
        );
        let (mut sending, mut wire, _unused) = DataStream::local(128 * 1024);
        let writer = async {
            sync.write(&mut sending).await.unwrap();
            sending.finish().unwrap();
        };
        let reader = async {
            let mut received = Vec::new();
            loop {
                let mut bytes = Vec::new();
                loop {
                    let length = wire.read_u32().await.unwrap();
                    if length == 0 {
                        break;
                    }
                    let start = bytes.len();
                    bytes.resize(start + length as usize, 0);
                    wire.read_exact(&mut bytes[start..]).await.unwrap();
                }
                let MemberSyncWire::Page {
                    member_sync_page,
                    continues,
                } = serde_json::from_slice(&bytes).unwrap()
                else {
                    panic!("missing page");
                };
                assert!(member_sync_page.bundle.management.len() <= MAX_RECORDS);
                received.extend(member_sync_page.bundle.management);
                if !continues {
                    assert_eq!(member_sync_page.bundle.services, vec![vec![3]]);
                    break;
                }
            }
            assert_eq!(received, records);
        };
        tokio::join!(writer, reader);

        let page = MemberSyncWire::Page {
            member_sync_page: MemberSync::new(
                ServiceBundle {
                    management: vec![(vec![1], None)],
                    services: vec![],
                },
                DeviceNames::default(),
                MemberSyncMode::Management,
            ),
            continues: true,
        };
        let bytes = serde_json::to_vec(&page).unwrap();
        assert!(serde_json::from_slice::<ServiceBundle>(&bytes).is_err());
        let (mut stream, _unused, mut input) = DataStream::local(4096);
        input.write_u32(bytes.len() as u32).await.unwrap();
        input.write_all(&bytes).await.unwrap();
        input.write_u32(0).await.unwrap();
        // Keep the connection open. Invalid first-page content must fail before
        // the receiver asks for another page or retains a growing prefix.
        let key = p2panda_core::SigningKey::generate();
        let anchor = NetworkAnchor {
            root: key.verifying_key(),
            genesis: Hash::from_bytes([0; 32]),
        };
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(1),
                MemberSync::read(&mut stream, anchor)
            )
            .await
            .unwrap()
            .is_err()
        );
    }

    #[tokio::test]
    async fn member_sync_pages_encoded_bytes_and_preserves_signed_records() {
        use crate::management_auth::ManagementExtension;
        use p2panda_auth::{
            Access, GroupsExtensionArgs,
            group::{GroupAction, GroupMember},
        };
        use p2panda_core::{Header, SigningKey};
        let root = SigningKey::generate();
        let member = SigningKey::generate();
        let administrator = SigningKey::generate();
        let body = vec![255; 2_200_000];
        let genesis = Header::builder()
            .body(&body)
            .build(
                &root,
                ManagementExtension {
                    version: 1,
                    log_id: [1; 16],
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        dependencies: vec![],
                        action: GroupAction::Create {
                            initial_members: vec![
                                (
                                    GroupMember::Individual(root.verifying_key()),
                                    Access::manage(),
                                ),
                                (
                                    GroupMember::Individual(administrator.verifying_key()),
                                    Access::manage(),
                                ),
                            ],
                        },
                    },
                },
            )
            .encode();
        let anchor = NetworkAnchor {
            root: root.verifying_key(),
            genesis: Hash::digest(&genesis),
        };
        let grant = Header::builder()
            .body(&body)
            .build(
                &root,
                ManagementExtension {
                    version: 1,
                    log_id: [2; 16],
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        dependencies: vec![anchor.genesis],
                        action: GroupAction::Add {
                            member: GroupMember::Individual(member.verifying_key()),
                            access: Access::read(),
                        },
                    },
                },
            )
            .encode();
        let records = vec![(genesis, Some(body.clone())), (grant, Some(body))];
        let sync = MemberSync::new(
            ServiceBundle {
                management: records.clone(),
                services: vec![],
            },
            DeviceNames::default(),
            MemberSyncMode::Management,
        );
        assert!(serde_json::to_vec(&sync).unwrap().len() > MAX_BUNDLE_BYTES);
        let received = transfer(&sync, anchor).await.unwrap();
        assert_eq!(received.bundle.management, records);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefix.sqlite");
        let saved = tokio::sync::Mutex::new(
            crate::management_store::ManagementStore::open(&path, anchor)
                .await
                .unwrap(),
        );
        let interrupted = transfer_resuming(&sync, anchor, Vec::new(), |page| {
            let saved = &saved;
            async move {
                saved
                    .lock()
                    .await
                    .apply_batch(&page)
                    .await
                    .map_err(|error| io::Error::other(error.to_string()))?;
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "lost page acknowledgement",
                ))
            }
        })
        .await;
        assert!(interrupted.is_err());
        saved.into_inner().close().await;
        let reopened = crate::management_store::ManagementStore::open(&path, anchor)
            .await
            .unwrap();
        let retained: ManagementRecords = reopened
            .applied_records()
            .await
            .unwrap()
            .into_iter()
            .map(|record| (record.header, record.body))
            .collect();
        assert_eq!(retained, records[..1]);
        let durable_ids: std::collections::BTreeSet<_> = retained
            .iter()
            .map(|(header, _)| Hash::digest(header))
            .collect();
        let saved = tokio::sync::Mutex::new(reopened);
        let resumed = transfer_resuming(&sync, anchor, retained, |page| {
            assert!(
                page.iter()
                    .all(|(header, _)| !durable_ids.contains(&Hash::digest(header))),
                "an already durable record was transmitted again"
            );
            let saved = &saved;
            async move {
                saved
                    .lock()
                    .await
                    .apply_batch(&page)
                    .await
                    .map_err(|error| io::Error::other(error.to_string()))
            }
        })
        .await
        .unwrap();
        saved.into_inner().close().await;
        assert_eq!(resumed.bundle.management, records);
        assert!(received.mode() == MemberSyncMode::Management);
        let wrong_network = NetworkAnchor {
            root: member.verifying_key(),
            genesis: anchor.genesis,
        };
        assert!(transfer(&sync, wrong_network).await.is_err());
        let merged = ServiceBundle::default()
            .reconcile(anchor, &received.bundle)
            .unwrap();
        let (auth, _) = merged.bundle.verify(anchor).unwrap();
        assert!(
            auth.nodes()
                .iter()
                .any(|(id, _)| *id == member.verifying_key())
        );
        let first_page = serde_json::to_vec(&MemberSyncWire::Page {
            member_sync_page: MemberSync::new(
                ServiceBundle {
                    management: records[..1].to_vec(),
                    services: vec![],
                },
                DeviceNames::default(),
                MemberSyncMode::Management,
            ),
            continues: true,
        })
        .unwrap();
        let (mut receiving, _unused, mut input) = DataStream::local(128 * 1024);
        let truncated = async {
            for chunk in first_page.chunks(crate::data_stream::CHUNK_BYTES) {
                input.write_u32(chunk.len() as u32).await.unwrap();
                input.write_all(chunk).await.unwrap();
            }
            input.write_u32(0).await.unwrap();
            input.shutdown().await.unwrap();
        };
        let (_, result) = tokio::join!(truncated, MemberSync::read(&mut receiving, anchor));
        assert!(
            result.is_err(),
            "a verified prefix is not a complete history"
        );
    }

    #[tokio::test]
    async fn member_sync_small_history_keeps_original_wire_shape() {
        let sync = MemberSync::new(
            ServiceBundle {
                management: vec![(vec![2], None)],
                services: vec![],
            },
            DeviceNames::default(),
            MemberSyncMode::Management,
        );
        let (mut stream, mut wire, _unused) = DataStream::local(4096);
        sync.write(&mut stream).await.unwrap();
        let length = wire.read_u32().await.unwrap();
        let mut bytes = vec![0; length as usize];
        wire.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, serde_json::to_vec(&sync).unwrap());
        assert_eq!(wire.read_u32().await.unwrap(), 0);
    }
}
