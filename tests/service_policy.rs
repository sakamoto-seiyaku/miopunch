//! Public signed-service SDK regressions. Actual hosts, network traffic, file
//! approval and independent saved reads are qualified by the runtime drivers.
use std::collections::BTreeSet;

use miopunch_native_probe::file_transfer::{FileError, FileOffer, FileTasks};
use miopunch_native_probe::{
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
    service_policy::{Service, ServiceAction, ServiceLedger, ServiceRule},
};
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey};

// Supplemental private storage transitions compile the exact source already
// frozen by the SDK receipt, with the same selected dependencies. The public
// SDK assertions below continue to exercise the immutable library pin.
#[allow(dead_code)]
#[path = "../src/data_stream.rs"]
pub mod data_stream;
#[allow(dead_code)]
#[path = "../src/file_transfer.rs"]
mod shared_file_tests;

struct CommittedStorage(std::path::PathBuf);
impl shared_file_tests::FileSink for CommittedStorage {
    fn write(&mut self, _: u64, _: Vec<u8>) -> shared_file_tests::FileFuture<'_, ()> {
        Box::pin(async { unreachable!("committed sink must not be written") })
    }
    fn save(&mut self) -> shared_file_tests::FileFuture<'_, ()> {
        Box::pin(async { unreachable!("committed sink must not be saved again") })
    }
    fn discard(&mut self) -> shared_file_tests::FileFuture<'_, ()> {
        Box::pin(async {
            std::fs::remove_file(&self.0).map_err(|_| shared_file_tests::FileError::SaveFailed)
        })
    }
}

#[tokio::test]
async fn later_cleanup_does_not_assign_an_old_receipt_to_a_rejected_new_offer() {
    use shared_file_tests as shared;
    let directory = std::env::temp_dir().join(format!(
        "miopunch-receipt-context-{}",
        Hash::digest(SigningKey::generate().as_bytes())
    ));
    std::fs::create_dir(&directory).unwrap();
    let offer = shared::FileOffer {
        id: "ef".repeat(16),
        name: "original.bin".into(),
        size: 0,
        sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
    };
    let path = directory.join(offer.saved_name());
    std::fs::write(&path, b"").unwrap();
    let tasks = shared::FileTasks::default();
    let receipt = shared::FileReceipt {
        id: offer.id.clone(),
        size: 0,
        sha256: offer.sha256.clone(),
        saved_name: offer.saved_name(),
    };
    let mut owner = tasks.begin_receive(&offer, "original-peer").unwrap();
    owner.preparing();
    tasks.finish(&offer.id, &Ok(receipt.clone()));
    drop(owner);
    for number in 0..shared::TASK_HISTORY {
        let id = format!("{number:032x}");
        tasks
            .begin(shared::FileTaskStatus::new(
                id.clone(),
                "send",
                "other".into(),
                "other.bin".into(),
                0,
            ))
            .unwrap();
        tasks.finish(&id, &Err(shared::FileError::Cancelled));
    }
    let mut partial = shared::PartialFile::new(
        offer.clone(),
        "original-peer".into(),
        CommittedStorage(path.clone()),
    );
    partial.receipt = Some(receipt);
    let mut changed = offer.clone();
    changed.name = "different.bin".into();
    let mut owner = tasks.begin_receive(&changed, "new-peer").unwrap();
    owner.preparing();
    assert!(
        !shared::reject_owned_partial(
            &mut partial,
            &tasks,
            &changed,
            "new-peer",
            shared::FileError::Rejected
        )
        .await
    );
    shared::discard_partial(&mut partial, &tasks, shared::FileError::Cancelled).await;
    let result = tasks.status_for(&offer.id).unwrap();
    let preserved = path.is_file();
    drop(owner);
    std::fs::remove_dir_all(&directory).unwrap();
    assert!(preserved, "later cleanup removed a committed file");
    assert_eq!(
        result.state,
        shared::FileState::Failed,
        "cleanup adopted a receipt for different metadata/peer"
    );
    assert!(result.receipt.is_none());
}

async fn file_connections() -> (
    iroh::Endpoint,
    iroh::Endpoint,
    iroh::endpoint::Connection,
    iroh::endpoint::Connection,
) {
    let server = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .alpns(vec![b"file-approval-regression".to_vec()])
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let client = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let connecting = client.connect(server.addr(), b"file-approval-regression");
    let accepting = async { server.accept().await.unwrap().await.unwrap() };
    let (outgoing, incoming) = tokio::join!(connecting, accepting);
    (client, server, outgoing.unwrap(), incoming)
}

fn file_offer() -> FileOffer {
    FileOffer {
        id: "ab".repeat(16),
        name: "guest.bin".into(),
        size: 11,
        sha256: "cd".repeat(32),
    }
}

#[test]
fn cancellation_during_initial_reservation_never_leaves_an_ownerless_task() {
    use std::sync::{Arc, Barrier};
    // Only public operations and actual competing threads; no scheduler hook
    // or interposed mutex. Each case has independent bounded task storage.
    let cases: Vec<_> = (0..2048).map(|_| Arc::new(FileTasks::default())).collect();
    let barrier = Arc::new(Barrier::new(2));
    std::thread::scope(|scope| {
        let cancelling_barrier = barrier.clone();
        let cancelling_cases = &cases;
        scope.spawn(move || {
            for tasks in cancelling_cases {
                cancelling_barrier.wait();
                while tasks.cancel(&file_offer().id) == Err(FileError::UnknownTask) {
                    std::hint::spin_loop();
                }
                cancelling_barrier.wait();
            }
        });
        for tasks in &cases {
            barrier.wait();
            drop(tasks.begin_receive(&file_offer(), "guest"));
            barrier.wait();
        }
    });
    let ownerless: Vec<_> = cases
        .iter()
        .enumerate()
        .filter_map(|(number, tasks)| {
            let task = tasks.status().pop().unwrap();
            (!task.state.terminal()).then_some((number, task.state))
        })
        .collect();
    assert!(
        ownerless.is_empty(),
        "cancelled reservations without a terminal owner: {ownerless:?}"
    );
}

#[tokio::test]
async fn one_approved_guest_offer_has_exactly_one_preparation_owner() {
    let (client, server, out, incoming) = file_connections().await;
    let tasks = FileTasks::default();
    let offer = file_offer();
    let first = tasks.begin_guest_receive(&offer, "guest").unwrap();
    let mut approval = Box::pin(first.wait_for_approval(&incoming));
    assert!(futures_util::poll!(approval.as_mut()).is_pending());
    assert!(tasks.decide_file(&offer.id, true).unwrap());
    assert_eq!(approval.await, Ok(()));
    let duplicate = tasks.begin_receive(&offer, "guest");
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    assert_eq!(
        duplicate.err(),
        Some(FileError::Busy),
        "a second open must not share the approved receive token"
    );
}

#[tokio::test]
async fn dropping_an_approved_reservation_before_storage_preparation_releases_its_slot() {
    let (client, server, out, incoming) = file_connections().await;
    let tasks = FileTasks::default();
    for number in 0..8 {
        let mut offer = file_offer();
        offer.id = format!("{number:032x}");
        let reservation = tasks.begin_guest_receive(&offer, "guest").unwrap();
        let mut approval = Box::pin(reservation.wait_for_approval(&incoming));
        assert!(futures_util::poll!(approval.as_mut()).is_pending());
        assert!(tasks.decide_file(&offer.id, true).unwrap());
        approval.await.unwrap();
        // An early platform refusal owns no sink. Dropping this reservation
        // must make the bounded slot terminal without a separate cleanup job.
        drop(reservation);
        let task = tasks
            .status()
            .into_iter()
            .find(|task| task.id == offer.id)
            .unwrap();
        assert!(task.state.terminal());
        assert_eq!(task.receipt, None);
        assert_eq!(task.bytes, 0);
    }
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn a_duplicate_waiter_cannot_join_or_mutate_the_pending_approval() {
    let (client, server, out, incoming) = file_connections().await;
    let tasks = FileTasks::default();
    let offer = file_offer();
    let reservation = tasks.begin_guest_receive(&offer, "guest").unwrap();
    let mut first = Box::pin(reservation.wait_for_approval(&incoming));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    let duplicate_result = tasks.begin_guest_receive(&offer, "guest").err();
    let state = tasks.status()[0].state;
    tasks.decide_file(&offer.id, false).unwrap();
    let _ = first.await;
    out.close(0u32.into(), b"done");
    incoming.close(0u32.into(), b"done");
    client.close().await;
    server.close().await;
    assert_eq!(
        state,
        miopunch_native_probe::file_transfer::FileState::AwaitingApproval
    );
    assert!(
        duplicate_result == Some(FileError::Busy),
        "duplicate waiters must be rejected before changing the task"
    );
}

struct Network {
    root: SigningKey,
    admin: SigningKey,
    provider: SigningKey,
    caller: SigningKey,
    anchor: NetworkAnchor,
    auth: ManagementAuth,
}
impl Network {
    fn new() -> Self {
        let root = SigningKey::generate();
        let admin = SigningKey::generate();
        let provider = SigningKey::generate();
        let caller = SigningKey::generate();
        let create = Header::builder().build(
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
                                GroupMember::Individual(admin.verifying_key()),
                                Access::manage(),
                            ),
                        ],
                    },
                },
            },
        );
        let anchor = NetworkAnchor {
            root: root.verifying_key(),
            genesis: create.hash(),
        };
        let mut auth = ManagementAuth::new(anchor);
        auth.accept(&create.encode()).unwrap();
        let mut network = Self {
            root,
            admin,
            provider,
            caller,
            anchor,
            auth,
        };
        for node in [
            network.provider.verifying_key(),
            network.caller.verifying_key(),
        ] {
            network.management(
                false,
                GroupAction::Add {
                    member: GroupMember::Individual(node),
                    access: Access::read(),
                },
            );
        }
        network
    }
    fn management(&mut self, root: bool, action: GroupAction<p2panda_core::VerifyingKey, ()>) {
        let key = if root { &self.root } else { &self.admin };
        let header = Header::builder().build(
            key,
            ManagementExtension {
                version: 1,
                log_id: *SigningKey::generate()
                    .as_bytes()
                    .first_chunk::<16>()
                    .unwrap(),
                group: GroupsExtensionArgs {
                    group_id: self.anchor.root,
                    dependencies: self.auth.tips(),
                    action,
                },
            },
        );
        self.auth.accept(&header.encode()).unwrap();
    }
    fn append(
        &self,
        ledger: &mut ServiceLedger,
        author: &SigningKey,
        action: ServiceAction,
    ) -> Hash {
        let bytes = ledger
            .author(self.anchor, &self.auth, author, action)
            .unwrap();
        let id = Hash::digest(&bytes);
        *ledger = ledger.merge(self.anchor, &self.auth, &[bytes]).unwrap();
        id
    }
    fn invite(&self, ledger: &mut ServiceLedger, guest: &SigningKey) -> (Hash, [u8; 32]) {
        let token = *SigningKey::generate().as_bytes();
        let id = self.append(
            ledger,
            &self.admin,
            ServiceAction::InviteGuest {
                provider: self.provider.verifying_key(),
                service: Service::Files,
                token_hash: Hash::digest(token),
                created: 100,
                expires: 200,
            },
        );
        assert!(
            ledger
                .redeem(id, &token, guest.verifying_key(), &self.auth, 150)
                .unwrap()
        );
        self.append(
            ledger,
            &self.admin,
            ServiceAction::BindGuest {
                invitation: id,
                guest: guest.verifying_key(),
            },
        );
        (id, token)
    }
}

#[test]
fn administrator_and_provider_edit_one_effective_rule_and_other_members_cannot() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let deny = ServiceRule {
        denied: BTreeSet::from([n.caller.verifying_key()]),
        ..Default::default()
    };
    let action = ServiceAction::SetRule {
        provider: n.provider.verifying_key(),
        service: Service::Files,
        previous: None,
        rule: deny.clone(),
    };
    assert!(
        ledger
            .author(n.anchor, &n.auth, &n.caller, action.clone())
            .is_err()
    );
    assert!(ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
    let revision = n.append(&mut ledger, &n.admin, action);
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
    assert_eq!(
        ledger.rule(n.provider.verifying_key(), Service::Files).rule,
        deny
    );
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: Some(revision),
            rule: Default::default(),
        },
    );
    assert!(ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn changed_signature_or_network_cannot_change_the_effective_rule() {
    let n = Network::new();
    let ledger = ServiceLedger::default();
    let action = ServiceAction::SetRule {
        provider: n.provider.verifying_key(),
        service: Service::Files,
        previous: None,
        rule: ServiceRule {
            members: false,
            ..Default::default()
        },
    };
    let record = ledger.author(n.anchor, &n.auth, &n.admin, action).unwrap();
    let mut changed = record.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(ledger.merge(n.anchor, &n.auth, &[changed]).is_err());
    let other = Network::new();
    assert!(ledger.merge(other.anchor, &other.auth, &[record]).is_err());
    assert!(ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn one_invitation_binds_one_guest_without_ordinary_membership_and_has_one_deadline() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let guest = SigningKey::generate();
    let (invitation, token) = n.invite(&mut ledger, &guest);
    assert!(
        !n.auth
            .nodes()
            .iter()
            .any(|(id, _)| *id == guest.verifying_key())
    );
    assert!(
        !ledger
            .redeem(invitation, &token, guest.verifying_key(), &n.auth, 199)
            .unwrap()
    );
    assert!(
        ledger
            .redeem(
                invitation,
                &token,
                SigningKey::generate().verifying_key(),
                &n.auth,
                199
            )
            .is_err()
    );
    assert!(ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        199
    ));
    assert!(!ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        200
    ));
    assert!(
        ledger
            .redeem(invitation, &token, guest.verifying_key(), &n.auth, 200)
            .is_err()
    );
    let bytes = serde_json::to_vec(ledger.records()).unwrap();
    let records: Vec<Vec<u8>> = serde_json::from_slice(&bytes).unwrap();
    let reopened = ServiceLedger::replay(n.anchor, &n.auth, &records).unwrap();
    assert_eq!(reopened.guests()[0].expires, 200);
    assert!(!reopened.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        200
    ));
}

#[test]
fn guests_cannot_expand_the_invited_node_service_or_effective_acl() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let guest = SigningKey::generate();
    n.invite(&mut ledger, &guest);
    assert!(!ledger.guest_access(
        n.admin.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
    assert!(!ledger.guest_access(
        n.provider.verifying_key(),
        Service::Web,
        guest.verifying_key(),
        &n.auth,
        150
    ));
    let restrict = ServiceAction::SetRule {
        provider: n.provider.verifying_key(),
        service: Service::Files,
        previous: None,
        rule: ServiceRule {
            guests: false,
            ..Default::default()
        },
    };
    assert!(
        ledger
            .author(n.anchor, &n.auth, &guest, restrict.clone())
            .is_err()
    );
    n.append(&mut ledger, &n.admin, restrict);
    assert!(!ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn guest_exit_is_signed_by_the_bound_identity_and_remains_ended_after_replay() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let guest = SigningKey::generate();
    let (invitation, token) = n.invite(&mut ledger, &guest);
    let exit = ServiceAction::EndGuest { invitation };
    assert!(
        ledger
            .author(n.anchor, &n.auth, &SigningKey::generate(), exit.clone())
            .is_err()
    );
    let exit_id = n.append(&mut ledger, &guest, exit);
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: Default::default(),
        },
    );
    let reopened = ServiceLedger::replay(n.anchor, &n.auth, ledger.records()).unwrap();
    assert_eq!(
        Hash::digest(reopened.guest_end_record(invitation).unwrap()),
        exit_id
    );
    assert!(!reopened.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
    assert!(
        reopened
            .redeem(invitation, &token, guest.verifying_key(), &n.auth, 150)
            .is_err()
    );
}

#[test]
fn restoring_administrator_status_does_not_revive_its_old_guest_invitation() {
    let mut n = Network::new();
    let mut ledger = ServiceLedger::default();
    let guest = SigningKey::generate();
    n.invite(&mut ledger, &guest);
    let admin = GroupMember::Individual(n.admin.verifying_key());
    n.management(
        true,
        GroupAction::Demote {
            member: admin.clone(),
            access: Access::read(),
        },
    );
    let demoted = ServiceLedger::replay(n.anchor, &n.auth, ledger.records()).unwrap();
    assert!(!demoted.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
    n.management(
        true,
        GroupAction::Promote {
            member: admin,
            access: Access::manage(),
        },
    );
    let restored = ServiceLedger::replay(n.anchor, &n.auth, ledger.records()).unwrap();
    assert!(!restored.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn a_stale_revision_cannot_overwrite_a_new_effective_rule() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let stale = ledger
        .author(
            n.anchor,
            &n.auth,
            &n.admin,
            ServiceAction::SetRule {
                provider: n.provider.verifying_key(),
                service: Service::Files,
                previous: None,
                rule: Default::default(),
            },
        )
        .unwrap();
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: ServiceRule {
                members: false,
                ..Default::default()
            },
        },
    );
    assert!(ledger.merge(n.anchor, &n.auth, &[stale]).is_err());
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn capability_declarations_belong_to_the_provider_and_do_not_enable_a_service() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let declare = ServiceAction::Declare {
        provider: n.provider.verifying_key(),
        service: Service::Files,
        previous: None,
        capable: true,
        enabled: false,
    };
    assert!(
        ledger
            .author(n.anchor, &n.auth, &n.admin, declare.clone())
            .is_err()
    );
    n.append(&mut ledger, &n.provider, declare);
    let declaration = ledger
        .declaration(n.provider.verifying_key(), Service::Files)
        .unwrap();
    assert!(declaration.capable && !declaration.enabled);
}

#[test]
fn an_explicit_member_allow_does_not_admit_an_unknown_identity_or_override_a_deny() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let unknown = SigningKey::generate();
    let rule = ServiceRule {
        members: false,
        allowed: BTreeSet::from([n.caller.verifying_key(), unknown.verifying_key()]),
        ..Default::default()
    };
    let revision = n.append(
        &mut ledger,
        &n.admin,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule,
        },
    );
    assert!(ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.admin.verifying_key(),
        &n.auth,
        150
    ));
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        unknown.verifying_key(),
        &n.auth,
        150
    ));
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: Some(revision),
            rule: ServiceRule {
                allowed: BTreeSet::from([n.caller.verifying_key()]),
                denied: BTreeSet::from([n.caller.verifying_key()]),
                ..Default::default()
            },
        },
    );
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn an_ordinary_provider_can_issue_a_guest_for_its_own_service() {
    let n = Network::new();
    let mut ledger = ServiceLedger::default();
    let guest = SigningKey::generate();
    let token = *SigningKey::generate().as_bytes();
    let action = ServiceAction::InviteGuest {
        provider: n.provider.verifying_key(),
        service: Service::Files,
        token_hash: Hash::digest(token),
        created: 100,
        expires: 200,
    };
    assert!(
        ledger
            .author(n.anchor, &n.auth, &n.caller, action.clone())
            .is_err()
    );
    let invitation = n.append(&mut ledger, &n.provider, action);
    assert!(
        ledger
            .redeem(invitation, &token, guest.verifying_key(), &n.auth, 150)
            .unwrap()
    );
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::BindGuest {
            invitation,
            guest: guest.verifying_key(),
        },
    );
    assert!(ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn a_demoted_remote_author_cannot_use_old_heads_to_extend_the_current_rule() {
    let mut n = Network::new();
    let old_auth = n.auth.clone();
    let mut ledger = ServiceLedger::default();
    n.management(
        true,
        GroupAction::Demote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::read(),
        },
    );
    let restricted = n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: ServiceRule {
                members: false,
                ..Default::default()
            },
        },
    );
    let stale = ledger.author(
        n.anchor,
        &old_auth,
        &n.admin,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: Some(restricted),
            rule: Default::default(),
        },
    );
    if let Ok(record) = stale {
        assert!(
            ledger.merge(n.anchor, &n.auth, &[record]).is_err(),
            "lost remote authority must not reopen actual member access"
        );
    }
    assert!(!ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn current_role_loss_rejects_a_new_incoming_effect_even_if_the_previous_rule_predates_it() {
    let mut n = Network::new();
    let mut ledger = ServiceLedger::default();
    let restricted = n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: ServiceRule {
                members: false,
                ..Default::default()
            },
        },
    );
    let old_auth = n.auth.clone();
    n.management(
        true,
        GroupAction::Demote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::read(),
        },
    );
    let stale = ledger
        .author(
            n.anchor,
            &old_auth,
            &n.admin,
            ServiceAction::SetRule {
                provider: n.provider.verifying_key(),
                service: Service::Files,
                previous: Some(restricted),
                rule: Default::default(),
            },
        )
        .unwrap();
    assert!(
        ledger.merge(n.anchor, &n.auth, &[stale]).is_err(),
        "a retained signing snapshot does not restore current daily authority"
    );
    let replayed = ServiceLedger::replay(n.anchor, &n.auth, ledger.records()).unwrap();
    assert!(!replayed.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn an_authorized_successor_can_carry_historical_service_dependencies_after_role_loss() {
    let mut n = Network::new();
    let mut ledger = ServiceLedger::default();
    let old = n.append(
        &mut ledger,
        &n.admin,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: Default::default(),
        },
    );
    n.management(
        true,
        GroupAction::Demote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::read(),
        },
    );
    let replayed = ServiceLedger::replay(n.anchor, &n.auth, ledger.records()).unwrap();
    assert!(replayed.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
    n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: Some(old),
            rule: ServiceRule {
                members: false,
                ..Default::default()
            },
        },
    );
    let merged = ServiceLedger::default()
        .merge(n.anchor, &n.auth, ledger.records())
        .unwrap();
    assert!(!merged.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn restoring_a_role_does_not_authorize_an_incoming_rule_signed_with_its_old_episode() {
    let mut n = Network::new();
    let mut ledger = ServiceLedger::default();
    let restricted = n.append(
        &mut ledger,
        &n.provider,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: None,
            rule: ServiceRule {
                members: false,
                ..Default::default()
            },
        },
    );
    let old_auth = n.auth.clone();
    n.management(
        true,
        GroupAction::Demote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::read(),
        },
    );
    n.management(
        true,
        GroupAction::Promote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::manage(),
        },
    );
    let old = ledger
        .author(
            n.anchor,
            &old_auth,
            &n.admin,
            ServiceAction::SetRule {
                provider: n.provider.verifying_key(),
                service: Service::Files,
                previous: Some(restricted),
                rule: Default::default(),
            },
        )
        .unwrap();
    assert!(ledger.merge(n.anchor, &n.auth, &[old]).is_err());
    n.append(
        &mut ledger,
        &n.admin,
        ServiceAction::SetRule {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            previous: Some(restricted),
            rule: Default::default(),
        },
    );
    assert!(ledger.allows(
        n.provider.verifying_key(),
        Service::Files,
        n.caller.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn stale_management_heads_cannot_bind_or_end_a_newer_guest_offer() {
    let mut n = Network::new();
    let old_auth = n.auth.clone();
    let mut ledger = ServiceLedger::default();
    n.management(
        true,
        GroupAction::Demote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::read(),
        },
    );
    n.management(
        true,
        GroupAction::Promote {
            member: GroupMember::Individual(n.admin.verifying_key()),
            access: Access::manage(),
        },
    );
    let token = *SigningKey::generate().as_bytes();
    let invitation = n.append(
        &mut ledger,
        &n.admin,
        ServiceAction::InviteGuest {
            provider: n.provider.verifying_key(),
            service: Service::Files,
            token_hash: Hash::digest(token),
            created: 100,
            expires: 200,
        },
    );
    let guest = SigningKey::generate();
    assert!(
        ledger
            .author(
                n.anchor,
                &old_auth,
                &n.admin,
                ServiceAction::BindGuest {
                    invitation,
                    guest: guest.verifying_key()
                }
            )
            .is_err()
    );
    n.append(
        &mut ledger,
        &n.admin,
        ServiceAction::BindGuest {
            invitation,
            guest: guest.verifying_key(),
        },
    );
    assert!(
        ledger
            .author(
                n.anchor,
                &old_auth,
                &n.admin,
                ServiceAction::EndGuest { invitation }
            )
            .is_err()
    );
    assert!(ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
    n.append(&mut ledger, &guest, ServiceAction::EndGuest { invitation });
    assert!(!ledger.guest_access(
        n.provider.verifying_key(),
        Service::Files,
        guest.verifying_key(),
        &n.auth,
        150
    ));
}

#[test]
fn retained_service_history_does_not_block_a_valid_strong_remove_correction() {
    use miopunch_native_probe::service_protocol::ServiceBundle;
    let mut n = Network::new();
    let common = n.auth.clone();
    let removed_provider = SigningKey::generate();
    n.management(
        false,
        GroupAction::Add {
            member: GroupMember::Individual(removed_provider.verifying_key()),
            access: Access::read(),
        },
    );
    let grant = n.auth.tips()[0];
    let mut ledger = ServiceLedger::default();
    n.append(
        &mut ledger,
        &removed_provider,
        ServiceAction::Declare {
            provider: removed_provider.verifying_key(),
            service: Service::Files,
            previous: None,
            capable: true,
            enabled: true,
        },
    );
    let retained = ServiceBundle {
        management: n
            .auth
            .headers()
            .iter()
            .cloned()
            .map(|header| (header, None))
            .collect(),
        services: ledger.records().to_vec(),
    };
    let demotion = Header::builder()
        .build(
            &n.root,
            ManagementExtension {
                version: 1,
                log_id: [93; 16],
                group: GroupsExtensionArgs {
                    group_id: n.anchor.root,
                    dependencies: common.tips(),
                    action: GroupAction::Demote {
                        member: GroupMember::Individual(n.admin.verifying_key()),
                        access: Access::read(),
                    },
                },
            },
        )
        .encode();
    let mut corrected = common;
    corrected.accept(&demotion).unwrap();
    let incoming = ServiceBundle {
        management: corrected
            .headers()
            .iter()
            .cloned()
            .map(|header| (header, None))
            .collect(),
        services: vec![],
    };
    let merged = retained.merged(n.anchor, &incoming).expect(
        "retained service declaration must not prevent receipt of the valid management correction",
    );
    let (auth, _) = merged.verify(n.anchor).unwrap();
    assert!(auth.filtered_operation(grant));
    assert!(
        !auth
            .nodes()
            .iter()
            .any(|(id, _)| *id == removed_provider.verifying_key())
    );
    assert!(
        !auth
            .nodes()
            .contains(&(n.admin.verifying_key(), Access::manage()))
    );
    // The obsolete declaration may also arrive for the first time with the
    // correction. Its rejection must not withhold verified management.
    let mut full = n.auth.clone();
    full.accept(&demotion).unwrap();
    let obsolete = ServiceBundle {
        management: full
            .headers()
            .iter()
            .cloned()
            .map(|header| (header, None))
            .collect(),
        services: retained.services.clone(),
    };
    let fresh = ServiceBundle {
        management: incoming.management,
        services: vec![],
    };
    let outcome = fresh.reconcile(n.anchor, &obsolete).unwrap();
    assert!(outcome.policy_error.is_some());
    let (auth, ledger) = outcome.bundle.verify(n.anchor).unwrap();
    assert!(auth.filtered_operation(grant));
    assert!(
        !auth
            .nodes()
            .iter()
            .any(|(id, _)| *id == removed_provider.verifying_key())
    );
    assert!(ledger.records().is_empty());
}
