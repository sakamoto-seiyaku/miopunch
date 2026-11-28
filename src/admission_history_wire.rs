//! Approved history delivery. Small histories keep their original frame;
//! long histories retain verified original records across explicit contacts.
use std::{
    io,
    sync::{Arc, OnceLock},
};

use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};

use crate::management_auth::{ManagementAuth, NetworkAnchor};
use crate::management_record::{
    HistoryRecord, MAX_PAGE_BYTES, MAX_PAGE_RECORDS, decode_raw_records, raw_records_page,
};

pub(crate) const SINGLE_HISTORY: u8 = 1;
pub(crate) const PAGED_HISTORY: u8 = 3;
const FRAME: ControlMessageFormat = ControlMessageFormat::LengthPrefixed32 {
    maximum: MAX_PAGE_BYTES,
};
const OFFER: ControlMessageFormat = ControlMessageFormat::LengthPrefixed16 { maximum: 256 };
const CURSOR: ControlMessageFormat = ControlMessageFormat::Fixed { length: 40 };
const OFFSET: ControlMessageFormat = ControlMessageFormat::Fixed { length: 8 };
const TARGET_RECORDS: usize = 128;
pub(crate) const TARGET_BYTES: usize = 1024 * 1024;
pub(crate) type Records = Vec<(Vec<u8>, Option<Vec<u8>>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct HistoryBinding {
    pub(crate) anchor: NetworkAnchor,
    pub(crate) provider: [u8; 32],
    pub(crate) request: [u8; 16],
    pub(crate) generation: u64,
}

/// Staged originals never participate in the node's active membership view.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct HistoryCheckpoint {
    pub(crate) binding: HistoryBinding,
    pub(crate) authorization: Hash,
    #[serde(with = "crate::management_record::stored_records")]
    pub(crate) records: Records,
    pub(crate) complete: bool,
    #[serde(skip)]
    prepared: Arc<OnceLock<PreparedHistory>>,
}

#[derive(Clone)]
struct PreparedHistory {
    auth: ManagementAuth,
    prefix: Prefix,
}

#[derive(Clone)]
struct Prefix(blake3::Hasher);

impl Default for Prefix {
    fn default() -> Self {
        let mut hash = blake3::Hasher::new();
        hash.update(b"miopunch/admission-history-prefix/1\0");
        Self(hash)
    }
}

impl Prefix {
    fn push(&mut self, id: Hash) {
        self.0.update(id.as_bytes());
    }
    fn extend(&mut self, records: &[(Vec<u8>, Option<Vec<u8>>)]) {
        for record in records {
            self.push(Hash::digest(&record.0));
        }
    }
}

impl HistoryCheckpoint {
    pub(crate) fn new(
        binding: HistoryBinding,
        authorization: Hash,
        records: Records,
        complete: bool,
    ) -> Self {
        Self {
            binding,
            authorization,
            records,
            complete,
            prepared: Arc::default(),
        }
    }

    /// Restore derived verification once at node load, outside contact clocks.
    pub(crate) fn prepare(&self) -> io::Result<()> {
        if self.prepared.get().is_some() {
            return Ok(());
        }
        if self.binding.generation > i64::MAX as u64 {
            return Err(invalid());
        }
        let mut auth = ManagementAuth::new(self.binding.anchor);
        let mut prefix = Prefix::default();
        for (header, body) in &self.records {
            let id = auth
                .accept_record(header, body.as_deref())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            prefix.push(id);
        }
        let _ = self.prepared.set(PreparedHistory { auth, prefix });
        Ok(())
    }

    fn remember(&mut self, auth: &ManagementAuth, prefix: &Prefix) {
        self.prepared = Arc::default();
        let _ = self.prepared.set(PreparedHistory {
            auth: auth.clone(),
            prefix: prefix.clone(),
        });
    }

    fn prepared(&self) -> io::Result<PreparedHistory> {
        self.prepare()?;
        Ok(self
            .prepared
            .get()
            .expect("prepared admission prefix")
            .clone())
    }
}

pub(crate) struct History<'a> {
    authorization: Hash,
    records: &'a [(Vec<u8>, Option<Vec<u8>>)],
    single: Option<Vec<u8>>,
}

impl<'a> History<'a> {
    pub(crate) fn new(
        authorization: Hash,
        records: &'a [(Vec<u8>, Option<Vec<u8>>)],
    ) -> io::Result<Self> {
        let mut history = Self::paged(authorization, records)?;
        // A history can fit the wire ceiling but take several contact windows
        // to verify. Start checkpointing at the ordinary page budget.
        if fits_single_page(records) {
            let encoded = p2panda_core::cbor::encode_cbor(&(authorization, records))
                .map_err(io::Error::other)?;
            history.single = (encoded.len() <= TARGET_BYTES).then_some(encoded);
        }
        Ok(history)
    }

    pub(crate) fn paged(
        authorization: Hash,
        records: &'a [(Vec<u8>, Option<Vec<u8>>)],
    ) -> io::Result<Self> {
        if records.is_empty() {
            return Err(invalid());
        }
        // Reject an original that cannot fit with its actual page envelope.
        for record in records {
            if record
                .byte_len()
                .checked_add(20)
                .is_none_or(|size| size > MAX_PAGE_BYTES)
            {
                return Err(invalid());
            }
        }
        Ok(Self {
            authorization,
            records,
            single: None,
        })
    }

    pub(crate) fn status(&self) -> u8 {
        if self.single.is_some() {
            SINGLE_HISTORY
        } else {
            PAGED_HISTORY
        }
    }

    pub(crate) async fn send(self, exchange: &ControlExchange) -> io::Result<()> {
        if let Some(bytes) = self.single {
            return exchange.send_message(FRAME, &bytes).await;
        }
        let offer =
            p2panda_core::cbor::encode_cbor(&(self.authorization, self.records.len() as u64))
                .map_err(io::Error::other)?;
        exchange.send_message(OFFER, &offer).await?;
        let requested = exchange.recv_message(CURSOR).await?;
        let wanted = usize::try_from(u64::from_be_bytes(requested[..8].try_into().unwrap()))
            .map_err(|_| invalid())?;
        let mut prefix = Prefix::default();
        if wanted <= self.records.len() {
            prefix.extend(&self.records[..wanted]);
        }
        let mut cursor =
            if wanted <= self.records.len() && cursor_bytes(wanted, &prefix) == requested {
                wanted
            } else {
                prefix = Prefix::default();
                0
            };
        exchange
            .send_message(OFFSET, &(cursor as u64).to_be_bytes())
            .await?;
        while cursor < self.records.len() {
            let (count, bytes) = data_page(cursor, &self.records[cursor..])?;
            exchange.send_message(FRAME, &bytes).await?;
            let ack = exchange.recv_message(CURSOR).await?;
            let next = cursor + count;
            prefix.extend(&self.records[cursor..next]);
            if ack != cursor_bytes(next, &prefix) {
                return Err(invalid());
            }
            cursor = next;
        }
        Ok(())
    }
}

pub(crate) fn fits_single_page(records: &[(Vec<u8>, Option<Vec<u8>>)]) -> bool {
    records.len() <= TARGET_RECORDS
        && records.iter().map(HistoryRecord::byte_len).sum::<usize>() <= TARGET_BYTES
}

fn data_page(
    offset: usize,
    remaining: &[(Vec<u8>, Option<Vec<u8>>)],
) -> io::Result<(usize, Vec<u8>)> {
    let mut count = 0;
    let mut bytes = 12usize; // offset and raw-record count
    for record in remaining.iter().take(TARGET_RECORDS) {
        let next = bytes
            .checked_add(8 + record.byte_len())
            .ok_or_else(invalid)?;
        if count > 0 && next > TARGET_BYTES {
            break;
        }
        count += 1;
        bytes = next;
    }
    let (count, raw) = raw_records_page(&remaining[..count], 8)?;
    if count == 0 {
        return Err(invalid());
    }
    let mut bytes = (offset as u64).to_be_bytes().to_vec();
    bytes.extend_from_slice(&raw);
    Ok((count, bytes))
}

fn cursor_bytes(count: usize, prefix: &Prefix) -> Vec<u8> {
    let mut bytes = (count as u64).to_be_bytes().to_vec();
    bytes.extend_from_slice(prefix.0.finalize().as_bytes());
    bytes
}

fn decode_single(bytes: &[u8]) -> io::Result<(Hash, Records)> {
    let (authorization, records): (Hash, Records) =
        p2panda_core::cbor::decode_cbor(bytes).map_err(|_| invalid())?;
    if records.is_empty() || records.len() > MAX_PAGE_RECORDS {
        return Err(invalid());
    }
    Ok((authorization, records))
}

pub(crate) async fn receive<Fut>(
    exchange: &ControlExchange,
    status: u8,
    binding: HistoryBinding,
    previous: Option<HistoryCheckpoint>,
    mut persist: impl FnMut(HistoryCheckpoint) -> Fut,
) -> io::Result<(Hash, Records, Option<ManagementAuth>)>
where
    Fut: std::future::Future<Output = io::Result<()>>,
{
    if status == SINGLE_HISTORY {
        let (authorization, records) = decode_single(&exchange.recv_message(FRAME).await?)?;
        return Ok((authorization, records, None));
    }
    if status != PAGED_HISTORY {
        return Err(invalid());
    }
    let (authorization, total): (Hash, u64) =
        p2panda_core::cbor::decode_cbor(exchange.recv_message(OFFER).await?.as_slice())
            .map_err(|_| invalid())?;
    let total = usize::try_from(total).map_err(|_| invalid())?;
    if total == 0 {
        return Err(invalid());
    }
    let mut checkpoint = previous
        .filter(|saved| saved.binding == binding && saved.authorization == authorization)
        .unwrap_or_else(|| HistoryCheckpoint::new(binding, authorization, Vec::new(), false));
    let PreparedHistory {
        auth: mut verified,
        mut prefix,
    } = checkpoint.prepared()?;
    exchange
        .send_message(CURSOR, &cursor_bytes(checkpoint.records.len(), &prefix))
        .await?;
    let accepted = usize::try_from(u64::from_be_bytes(
        exchange.recv_message(OFFSET).await?.try_into().unwrap(),
    ))
    .map_err(|_| invalid())?;
    if accepted != checkpoint.records.len() {
        if accepted != 0 {
            return Err(invalid());
        }
        checkpoint.records.clear();
        checkpoint.complete = false;
        verified = ManagementAuth::new(binding.anchor);
        prefix = Prefix::default();
        checkpoint.remember(&verified, &prefix);
        persist(checkpoint.clone()).await?;
    }
    if accepted > total {
        return Err(invalid());
    }
    while checkpoint.records.len() < total {
        let bytes = exchange.recv_message(FRAME).await?;
        let start = bytes.get(..8).ok_or_else(invalid)?;
        let offset = usize::try_from(u64::from_be_bytes(start.try_into().unwrap()))
            .map_err(|_| invalid())?;
        let mut page = decode_raw_records(&bytes[8..])?;
        if offset != checkpoint.records.len()
            || page.is_empty()
            || offset.checked_add(page.len()).is_none_or(|end| end > total)
        {
            return Err(invalid());
        }
        for (header, body) in &page {
            let id = verified
                .accept_record(header, body.as_deref())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            prefix.push(id);
        }
        checkpoint.records.append(&mut page);
        checkpoint.complete = checkpoint.records.len() == total;
        checkpoint.remember(&verified, &prefix);
        // Cursor and original bytes commit together before acknowledging them.
        persist(checkpoint.clone()).await?;
        exchange
            .send_message(CURSOR, &cursor_bytes(checkpoint.records.len(), &prefix))
            .await?;
    }
    Ok((authorization, checkpoint.records, Some(verified)))
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid admission history page or cursor",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_history() -> (
        NetworkAnchor,
        Records,
        p2panda_core::SigningKey,
        p2panda_core::SigningKey,
    ) {
        use crate::management_auth::ManagementExtension;
        use p2panda_auth::{
            Access, GroupsExtensionArgs,
            group::{GroupAction, GroupMember},
        };
        use p2panda_core::{Header, SigningKey};
        let root = SigningKey::generate();
        let administrator = SigningKey::generate();
        let genesis = Header::<ManagementExtension>::builder()
            .build(
                &root,
                ManagementExtension {
                    version: 1,
                    log_id: [0; 16],
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
        let mut previous = anchor.genesis;
        let mut records = vec![(genesis, None)];
        for index in 1..130u128 {
            let header = Header::<ManagementExtension>::builder()
                .build(
                    &root,
                    ManagementExtension {
                        version: 1,
                        log_id: index.to_be_bytes(),
                        group: GroupsExtensionArgs {
                            group_id: root.verifying_key(),
                            dependencies: vec![previous],
                            action: GroupAction::Promote {
                                member: GroupMember::Individual(administrator.verifying_key()),
                                access: Access::manage(),
                            },
                        },
                    },
                )
                .encode();
            previous = Hash::digest(&header);
            records.push((header, None));
        }
        (anchor, records, root, administrator)
    }

    #[test]
    fn prefix_identity_distinguishes_legal_equal_length_branches_with_the_same_tail() {
        use crate::management_auth::ManagementExtension;
        use p2panda_auth::{
            Access, GroupsExtensionArgs,
            group::{GroupAction, GroupMember},
        };
        use p2panda_core::Header;
        let (anchor, records, root, administrator) = signed_history();
        let independent = |index: u128| {
            (
                Header::<ManagementExtension>::builder()
                    .build(
                        &root,
                        ManagementExtension {
                            version: 1,
                            log_id: index.to_be_bytes(),
                            group: GroupsExtensionArgs {
                                group_id: anchor.root,
                                dependencies: vec![anchor.genesis],
                                action: GroupAction::Promote {
                                    member: GroupMember::Individual(administrator.verifying_key()),
                                    access: Access::manage(),
                                },
                            },
                        },
                    )
                    .encode(),
                None,
            )
        };
        let tail = independent(1003);
        let left = vec![records[0].clone(), independent(1001), tail.clone()];
        let right = vec![records[0].clone(), independent(1002), tail];
        ManagementAuth::replay_records(anchor, &left).unwrap();
        ManagementAuth::replay_records(anchor, &right).unwrap();
        assert_eq!(left.len(), right.len());
        assert_eq!(left.last(), right.last());
        let mut a = Prefix::default();
        a.extend(&left[..1]);
        a.extend(&left[1..]);
        let mut b = Prefix::default();
        b.extend(&right);
        assert_ne!(cursor_bytes(left.len(), &a), cursor_bytes(right.len(), &b));
        let mut whole = Prefix::default();
        whole.extend(&left);
        assert_eq!(
            cursor_bytes(left.len(), &a),
            cursor_bytes(left.len(), &whole)
        );
    }

    #[tokio::test]
    async fn verified_prefix_survives_reopen_and_resumes_only_the_remaining_originals() {
        use crate::management_store::ManagementStore;
        use iroh::{
            Endpoint, EndpointAddr,
            endpoint::{Connection, presets},
        };
        use std::{net::Ipv4Addr, time::Duration};
        let (anchor, records, _, _) = signed_history();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("staging.sqlite");
        let store = ManagementStore::open(&path, anchor).await.unwrap();
        let bind = || async {
            Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .alpns(vec![b"admission-history-test".to_vec()])
                .bind()
                .await
                .unwrap()
        };
        let (client, server) = tokio::join!(bind(), bind());
        let address = EndpointAddr::new(server.id()).with_addrs(server.addr().addrs);
        let (outgoing, incoming) =
            tokio::join!(client.connect(address, b"admission-history-test"), async {
                server.accept().await.unwrap().await
            });
        let (outgoing, incoming) = (outgoing.unwrap(), incoming.unwrap());
        let binding = HistoryBinding {
            anchor,
            provider: *server.id().as_bytes(),
            request: [7; 16],
            generation: 0,
        };

        async fn transfer(
            outgoing: &Connection,
            incoming: &Connection,
            store: &ManagementStore,
            binding: HistoryBinding,
            previous: Option<HistoryCheckpoint>,
            records: &Records,
            stop: bool,
        ) -> io::Result<(Hash, Records, Option<ManagementAuth>)> {
            let sending = async {
                let (send, recv) = outgoing.open_bi().await.map_err(io::Error::other)?;
                let exchange = ControlExchange::from_streams(send, recv);
                let result = History {
                    authorization: binding.anchor.genesis,
                    records,
                    single: None,
                }
                .send(&exchange)
                .await;
                if result.is_ok() {
                    exchange
                        .send_message(ControlMessageFormat::Fixed { length: 1 }, &[1])
                        .await?;
                    exchange.finish_send().await?;
                }
                result
            };
            let receiving = async {
                let (send, recv) = incoming.accept_bi().await.map_err(io::Error::other)?;
                let exchange = ControlExchange::from_streams(send, recv);
                let result = receive(
                    &exchange,
                    PAGED_HISTORY,
                    binding,
                    previous,
                    move |checkpoint| async move {
                        store
                            .save_admission_history(&checkpoint)
                            .await
                            .map_err(io::Error::other)?;
                        if stop {
                            return Err(io::ErrorKind::Interrupted.into());
                        }
                        Ok(())
                    },
                )
                .await;
                if result.is_ok() {
                    // Production retains this exchange for AUTH/secret delivery.
                    // Keep its final page ACK live until the sender consumes it.
                    exchange
                        .recv_message(ControlMessageFormat::Fixed { length: 1 })
                        .await?;
                }
                result
            };
            let (sent, received) = tokio::join!(sending, receiving);
            if stop {
                assert!(sent.is_err());
            } else {
                sent?;
            }
            received
        }
        let stopped = tokio::time::timeout(
            Duration::from_secs(8),
            transfer(&outgoing, &incoming, &store, binding, None, &records, true),
        )
        .await
        .unwrap();
        assert!(stopped.is_err());
        let saved = store.admission_history_checkpoint().await.unwrap().unwrap();
        assert_eq!(saved.records, records[..TARGET_RECORDS]);
        assert!(!saved.complete);
        assert_eq!(
            store.applied_count(),
            0,
            "staged authorization is not an active member view"
        );
        store.close().await;
        let store = ManagementStore::open(&path, anchor).await.unwrap();
        let previous = store.admission_history_checkpoint().await.unwrap().unwrap();
        assert_eq!(previous.binding, binding);
        assert_eq!(previous.records, saved.records);
        let received = tokio::time::timeout(
            Duration::from_secs(8),
            transfer(
                &outgoing,
                &incoming,
                &store,
                binding,
                Some(previous),
                &records,
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(received.1, records);
        assert_eq!(received.2.unwrap().headers().len(), records.len());
        let completed = store.admission_history_checkpoint().await.unwrap().unwrap();
        assert!(completed.complete);
        assert_eq!(completed.records, records);
        assert_eq!(store.applied_count(), 0);
        store.clear_admission_history().await.unwrap();
        assert!(
            store
                .admission_history_checkpoint()
                .await
                .unwrap()
                .is_none()
        );
        store.close().await;
        client.close().await;
        server.close().await;
    }

    #[test]
    fn small_history_preserves_the_original_payload_and_status() {
        let records = vec![(vec![1, 2], None::<Vec<u8>>)];
        let authorization = Hash::from_bytes([7; 32]);
        let history = History::new(authorization, &records).unwrap();
        assert_eq!(history.status(), SINGLE_HISTORY);
        assert_eq!(
            history.single.unwrap(),
            p2panda_core::cbor::encode_cbor(&(authorization, &records)).unwrap()
        );
    }

    #[test]
    fn compact_pages_cover_every_original_with_the_real_envelope_limit() {
        let pattern: Vec<u8> = (0..=255).collect();
        for records in [
            vec![(pattern.clone(), None); TARGET_RECORDS + 1],
            vec![(pattern.clone(), None); MAX_PAGE_RECORDS + 1],
            vec![(vec![1; MAX_PAGE_BYTES / 2], None); 3],
        ] {
            assert_eq!(
                History::new(Hash::from_bytes([7; 32]), &records)
                    .unwrap()
                    .status(),
                PAGED_HISTORY
            );
            let mut offset = 0;
            while offset < records.len() {
                let (count, page) = data_page(offset, &records[offset..]).unwrap();
                assert!(count <= TARGET_RECORDS && page.len() <= MAX_PAGE_BYTES);
                if count > 1 {
                    assert!(page.len() <= TARGET_BYTES);
                }
                let decoded = decode_raw_records(&page[8..]).unwrap();
                assert_eq!(decoded, records[offset..offset + count]);
                assert_eq!(
                    page.len(),
                    12 + decoded.iter().map(|r| 8 + r.byte_len()).sum::<usize>()
                );
                offset += count;
            }
        }
        assert!(
            History::new(
                Hash::from_bytes([7; 32]),
                &[(vec![0; MAX_PAGE_BYTES], None)]
            )
            .is_err()
        );
    }
}
