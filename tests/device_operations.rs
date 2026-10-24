//! Public domain invariants behind the real Native/Browser device operations.
use miopunch_native_probe::{
    credential_custody::{
        CustodyJournal, CustodyOffer, CustodyProbe, CustodyProof, CustodyReceipt,
    },
    device_operation::{DeviceAction, DeviceOperation, OperationJournal},
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
};
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};

fn record(
    author: &SigningKey,
    anchor: VerifyingKey,
    action: GroupAction<VerifyingKey>,
    dependencies: Vec<Hash>,
    tag: u8,
) -> Vec<u8> {
    Header::<ManagementExtension>::builder()
        .build(
            author,
            ManagementExtension {
                version: 1,
                log_id: [tag; 16],
                group: GroupsExtensionArgs {
                    group_id: anchor,
                    action,
                    dependencies,
                },
            },
        )
        .encode()
}

fn network() -> (
    SigningKey,
    SigningKey,
    SigningKey,
    NetworkAnchor,
    ManagementAuth,
) {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let member = SigningKey::generate();
    let create = record(
        &root,
        root.verifying_key(),
        GroupAction::Create {
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
        vec![],
        1,
    );
    let anchor = NetworkAnchor {
        root: root.verifying_key(),
        genesis: Hash::digest(&create),
    };
    let mut auth = ManagementAuth::replay(anchor, &[create]).unwrap();
    let add = record(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: GroupMember::Individual(member.verifying_key()),
            access: Access::read(),
        },
        auth.tips(),
        2,
    );
    auth.accept(&add).unwrap();
    (root, admin, member, anchor, auth)
}

#[test]
fn a_specific_approval_cannot_authorize_another_description_or_network() {
    let (root, admin, member, anchor, auth) = network();
    let operation = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap();
    assert!(operation.approve(anchor, &auth, &admin).is_err());
    let approval = operation.approve(anchor, &auth, &root).unwrap();
    approval.verify(&operation, anchor).unwrap();
    let another = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap();
    assert_ne!(operation.id, another.id);
    assert!(approval.verify(&another, anchor).is_err());
    let removal = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::RemoveMember,
        member.verifying_key(),
    )
    .unwrap();
    assert!(approval.verify(&removal, anchor).is_err());
    let mut wrong_network = anchor;
    wrong_network.genesis = Hash::digest(b"different original create");
    assert!(approval.verify(&operation, wrong_network).is_err());
    let mut changed = approval.clone();
    changed.signature[0] ^= 1;
    assert!(changed.verify(&operation, anchor).is_err());
}

#[test]
fn pending_confirmation_and_custody_offer_do_not_survive_removed_rejoined_episode() {
    let (root, admin, member, anchor, mut auth) = network();
    let operation = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap();
    let offer = CustodyOffer::new(
        anchor,
        &auth,
        admin.verifying_key(),
        member.verifying_key(),
        &root,
    )
    .unwrap();
    let remove = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::RemoveMember,
        member.verifying_key(),
    )
    .unwrap();
    auth.accept(&remove.approve(anchor, &auth, &admin).unwrap().record)
        .unwrap();
    let rejoin = record(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: GroupMember::Individual(member.verifying_key()),
            access: Access::read(),
        },
        auth.tips(),
        3,
    );
    auth.accept(&rejoin).unwrap();
    assert!(
        auth.nodes()
            .contains(&(member.verifying_key(), Access::read()))
    );
    assert!(operation.check_current(anchor, &auth).is_err());
    assert!(operation.approve(anchor, &auth, &root).is_err());
    assert!(offer.check_current(anchor, &auth).is_err());
    DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap()
    .approve(anchor, &auth, &root)
    .unwrap();
}

#[test]
fn queued_ordinary_removal_or_leave_cannot_bypass_a_later_administrator_role() {
    let (root, admin, member, anchor, mut auth) = network();
    let remove = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::RemoveMember,
        member.verifying_key(),
    )
    .unwrap();
    let leave = DeviceOperation::propose(
        anchor,
        &auth,
        member.verifying_key(),
        DeviceAction::Leave,
        member.verifying_key(),
    )
    .unwrap();
    assert!(!remove.requires_genesis() && !leave.requires_genesis());
    let appoint = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap()
    .approve(anchor, &auth, &root)
    .unwrap();
    auth.accept(&appoint.record).unwrap();
    assert!(remove.approve(anchor, &auth, &admin).is_err());
    assert!(leave.approve(anchor, &auth, &member).is_err());
    for (initiator, action) in [
        (&admin, DeviceAction::RemoveMember),
        (&member, DeviceAction::Leave),
    ] {
        let fresh = DeviceOperation::propose(
            anchor,
            &auth,
            initiator.verifying_key(),
            action,
            member.verifying_key(),
        )
        .unwrap();
        assert!(fresh.requires_genesis());
        assert!(fresh.approve(anchor, &auth, initiator).is_err());
        fresh.approve(anchor, &auth, &root).unwrap();
    }
}

#[test]
fn original_result_remains_queryable_after_later_demotion_without_reexecuting() {
    let (root, admin, member, anchor, mut auth) = network();
    let operation = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::AppointAdministrator,
        member.verifying_key(),
    )
    .unwrap();
    let approval = operation.approve(anchor, &auth, &root).unwrap();
    let original = approval.record_id();
    let mut journal = OperationJournal::default();
    journal.retain(operation.clone()).unwrap();
    auth.accept(&approval.record).unwrap();
    let receipt = auth
        .headers()
        .iter()
        .cloned()
        .map(|header| (header, None))
        .collect::<Vec<_>>();
    journal.approve(approval.clone(), receipt.clone()).unwrap();
    journal.approve(approval.clone(), receipt).unwrap();
    let demotion = DeviceOperation::propose(
        anchor,
        &auth,
        admin.verifying_key(),
        DeviceAction::DemoteAdministrator,
        member.verifying_key(),
    )
    .unwrap();
    auth.accept(&demotion.approve(anchor, &auth, &root).unwrap().record)
        .unwrap();
    assert!(!approval.effect_current(&auth));
    assert_eq!(
        journal
            .get(operation.id)
            .unwrap()
            .approval
            .as_ref()
            .unwrap()
            .record_id(),
        original
    );
    journal.verify(anchor).unwrap();
    let restored: OperationJournal =
        serde_json::from_str(&serde_json::to_string(&journal).unwrap()).unwrap();
    restored.verify(anchor).unwrap();
    assert_eq!(restored.entries.len(), 1);
    assert_eq!(
        restored
            .get(operation.id)
            .unwrap()
            .approval
            .as_ref()
            .unwrap()
            .record_id(),
        original
    );
}

#[test]
fn custody_evidence_binds_recipient_network_and_fresh_challenge() {
    let (root, admin, member, anchor, auth) = network();
    let mut before = auth.nodes();
    before.sort_by_key(|(member, _)| *member.as_bytes());
    let offer = CustodyOffer::new(
        anchor,
        &auth,
        admin.verifying_key(),
        member.verifying_key(),
        &root,
    )
    .unwrap();
    let receipt = CustodyReceipt::saved(&offer, &root, &member).unwrap();
    receipt.verify(&offer).unwrap();
    assert!(CustodyReceipt::saved(&offer, &root, &admin).is_err());
    let another = CustodyOffer::new(
        anchor,
        &auth,
        admin.verifying_key(),
        member.verifying_key(),
        &root,
    )
    .unwrap();
    assert!(receipt.verify(&another).is_err());
    let probe = CustodyProbe {
        network: anchor,
        member: member.verifying_key(),
        member_authorization: auth
            .authorization_origin_at(member.verifying_key(), &auth.tips(), false)
            .unwrap(),
        challenge: [1; 32],
    };
    let proof = CustodyProof::sign(probe.clone(), &root).unwrap();
    proof.verify(&probe).unwrap();
    let mut fresh = probe;
    fresh.challenge[0] ^= 1;
    assert!(proof.verify(&fresh).is_err());
    let mut after = auth.nodes();
    after.sort_by_key(|(member, _)| *member.as_bytes());
    assert_eq!(after, before, "custody evidence must not change membership or roles");
}

#[test]
fn stopping_custody_preserves_history_and_requires_a_fresh_recipient_decision() {
    let (root, admin, member, anchor, auth) = network();
    let recipient = member.verifying_key();
    let offer =
        || CustodyOffer::new(anchor, &auth, admin.verifying_key(), recipient, &root).unwrap();
    let pending = offer();
    let completed = offer();
    let unconfirmed = offer();
    let outgoing =
        CustodyOffer::new(anchor, &auth, recipient, admin.verifying_key(), &root).unwrap();
    let mut journal = CustodyJournal::default();
    for offer in [&pending, &completed, &unconfirmed, &outgoing] {
        journal.retain(offer.clone()).unwrap();
    }
    journal.get_mut(pending.id).unwrap().accept().unwrap();
    journal.get_mut(completed.id).unwrap().accept().unwrap();
    let receipt = CustodyReceipt::saved(&completed, &root, &member).unwrap();
    journal.get_mut(completed.id).unwrap().receipt = Some(receipt.clone());
    journal.stop_received(recipient);
    journal.stop_received(recipient);
    journal.verify(anchor).unwrap();
    // A normal serialized reopen preserves the local stop, including offers
    // that had not yet been accepted when the user chose to stop custody.
    let mut reopened: CustodyJournal =
        serde_json::from_slice(&serde_json::to_vec(&journal).unwrap()).unwrap();
    reopened.verify(anchor).unwrap();
    for id in [pending.id, unconfirmed.id] {
        let entry = reopened.get_mut(id).unwrap();
        assert!(entry.stopped);
        assert!(!entry.recipient_confirmed());
        assert!(entry.accept().is_err());
        assert!(entry.delivery().is_err());
    }
    let historical = reopened.get_mut(completed.id).unwrap();
    assert!(historical.stopped);
    assert!(historical.accept().is_err());
    assert_eq!(historical.delivery().unwrap(), Some(&receipt));
    assert_eq!(historical.receipt.as_ref(), Some(&receipt));
    assert!(!reopened.get(outgoing.id).unwrap().stopped);
    let fresh = offer();
    reopened.retain(fresh.clone()).unwrap();
    assert!(reopened.get(fresh.id).unwrap().delivery().is_err());
    reopened.get_mut(fresh.id).unwrap().accept().unwrap();
    assert!(reopened.get(fresh.id).unwrap().recipient_confirmed());
    assert_eq!(reopened.get(fresh.id).unwrap().delivery().unwrap(), None);
    reopened.verify(anchor).unwrap();
}

#[test]
fn confirmation_queries_share_ordinary_and_genesis_result_eligibility() {
    let (_root, _admin, member, anchor, auth) = network();
    let operation = DeviceOperation::propose(
        anchor,
        &auth,
        member.verifying_key(),
        DeviceAction::Leave,
        member.verifying_key(),
    )
    .unwrap();
    let mut journal = OperationJournal::default();
    journal.retain(operation.clone()).unwrap();
    assert!(journal.get(operation.id).unwrap().confirmation().is_err());
    let approval = operation.approve(anchor, &auth, &member).unwrap();
    let mut completed_auth = auth.clone();
    completed_auth.accept(&approval.record).unwrap();
    let history = completed_auth
        .headers()
        .iter()
        .cloned()
        .map(|r| (r, None))
        .collect();
    journal.approve(approval.clone(), history).unwrap();
    let reopened: OperationJournal =
        serde_json::from_slice(&serde_json::to_vec(&journal).unwrap()).unwrap();
    reopened.verify(anchor).unwrap();
    assert_eq!(
        reopened.get(operation.id).unwrap().confirmation().unwrap(),
        Some(&approval)
    );
}
