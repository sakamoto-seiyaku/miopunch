use miopunch_native_probe::management_auth::{
    AuthError, ManagementAuth, ManagementExtension, NetworkAnchor,
};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};

fn individual(key: &SigningKey) -> GroupMember<VerifyingKey> {
    GroupMember::Individual(key.verifying_key())
}

fn signed(
    author: &SigningKey,
    root: VerifyingKey,
    action: GroupAction<VerifyingKey>,
    dependencies: Vec<Hash>,
    log_id: u8,
) -> Vec<u8> {
    Header::<ManagementExtension>::builder()
        .build(
            author,
            ManagementExtension {
                version: 1,
                log_id: [log_id; 16],
                group: GroupsExtensionArgs {
                    group_id: root,
                    action,
                    dependencies,
                },
            },
        )
        .encode()
}

fn genesis(root: &SigningKey, admin: &SigningKey) -> (NetworkAnchor, Vec<u8>) {
    let bytes = signed(
        root,
        root.verifying_key(),
        GroupAction::Create {
            initial_members: vec![
                (individual(root), Access::manage()),
                (individual(admin), Access::manage()),
            ],
        },
        vec![],
        1,
    );
    (
        NetworkAnchor {
            root: root.verifying_key(),
            genesis: Hash::digest(&bytes),
        },
        bytes,
    )
}

#[test]
fn signed_daily_approval_succeeds_but_admin_appointment_is_rejected() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let ordinary = SigningKey::generate();
    let candidate_admin = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();

    let ordinary_add = signed(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: individual(&ordinary),
            access: Access::read(),
        },
        vec![create_id],
        2,
    );
    let ordinary_id = auth.accept(&ordinary_add).unwrap();
    assert!(
        auth.nodes()
            .iter()
            .any(|(id, access)| { *id == ordinary.verifying_key() && *access == Access::read() })
    );
    assert!(!auth.nodes().iter().any(|(id, _)| *id == anchor.root));

    let appoint = signed(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: individual(&candidate_admin),
            access: Access::manage(),
        },
        vec![ordinary_id],
        3,
    );
    assert!(matches!(
        auth.accept(&appoint),
        Err(AuthError::PolicyRejected(_))
    ));
    assert!(
        !auth
            .nodes()
            .iter()
            .any(|(id, _)| *id == candidate_admin.verifying_key())
    );
    assert_eq!(auth.headers().len(), 2);
}

#[test]
fn concurrent_root_demote_filters_daily_remove_in_both_arrival_orders() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let target = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let create_id = Hash::digest(&create);
    let approve = signed(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: individual(&target),
            access: Access::read(),
        },
        vec![create_id],
        2,
    );
    let approve_id = Hash::digest(&approve);
    let demote = signed(
        &root,
        anchor.root,
        GroupAction::Demote {
            member: individual(&admin),
            access: Access::read(),
        },
        vec![approve_id],
        3,
    );
    let demote_id = Hash::digest(&demote);
    let remove = signed(
        &admin,
        anchor.root,
        GroupAction::Remove {
            member: individual(&target),
        },
        vec![approve_id],
        4,
    );
    let remove_id = Hash::digest(&remove);

    for concurrent in [[&remove, &demote], [&demote, &remove]] {
        let mut auth = ManagementAuth::new(anchor);
        auth.accept(&create).unwrap();
        auth.accept(&approve).unwrap();
        auth.accept(concurrent[0]).unwrap();
        auth.accept(concurrent[1]).unwrap();
        assert!(auth.contains_operation(remove_id));
        assert!(auth.filtered_operation(remove_id));
        assert!(
            auth.nodes()
                .iter()
                .any(|(id, access)| { *id == target.verifying_key() && *access == Access::read() })
        );
        assert!(
            auth.nodes()
                .iter()
                .any(|(id, access)| { *id == admin.verifying_key() && *access == Access::read() })
        );

        let replayed = ManagementAuth::replay(anchor, auth.headers()).unwrap();
        assert!(replayed.filtered_operation(remove_id));
        assert_eq!(replayed.nodes().len(), 2);

        let reappoint = signed(
            &root,
            anchor.root,
            GroupAction::Promote {
                member: individual(&admin),
                access: Access::manage(),
            },
            vec![demote_id, remove_id],
            5,
        );
        let reappoint_id = auth.accept(&reappoint).unwrap();
        let new_remove = signed(
            &admin,
            anchor.root,
            GroupAction::Remove {
                member: individual(&target),
            },
            vec![reappoint_id],
            6,
        );
        auth.accept(&new_remove).unwrap();
        assert!(
            !auth
                .nodes()
                .iter()
                .any(|(id, _)| *id == target.verifying_key())
        );
    }
}

#[test]
fn daily_manager_cannot_change_admins_or_exit_without_root_authority() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let ordinary = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();
    let add = signed(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: individual(&ordinary),
            access: Access::read(),
        },
        vec![create_id],
        2,
    );
    let add_id = auth.accept(&add).unwrap();

    for (tag, action) in [
        (
            3,
            GroupAction::Promote {
                member: individual(&ordinary),
                access: Access::manage(),
            },
        ),
        (
            4,
            GroupAction::Demote {
                member: individual(&admin),
                access: Access::read(),
            },
        ),
        (
            5,
            GroupAction::Remove {
                member: individual(&admin),
            },
        ),
        (
            6,
            GroupAction::Remove {
                member: individual(&root),
            },
        ),
    ] {
        let bytes = signed(&admin, anchor.root, action, vec![add_id], tag);
        assert!(matches!(
            auth.accept(&bytes),
            Err(AuthError::PolicyRejected(_))
        ));
    }
    assert_eq!(auth.headers().len(), 2);

    let remove_ordinary = signed(
        &admin,
        anchor.root,
        GroupAction::Remove {
            member: individual(&ordinary),
        },
        vec![add_id],
        7,
    );
    auth.accept(&remove_ordinary).unwrap();
    assert!(
        !auth
            .nodes()
            .iter()
            .any(|(id, _)| *id == ordinary.verifying_key())
    );
}

#[test]
fn alternate_create_wrong_root_and_tampered_signature_never_enter_graph() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let other = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();

    let alternate = signed(
        &root,
        anchor.root,
        GroupAction::Create {
            initial_members: vec![
                (individual(&root), Access::manage()),
                (individual(&other), Access::manage()),
            ],
        },
        vec![],
        8,
    );
    assert!(matches!(
        auth.accept(&alternate),
        Err(AuthError::PolicyRejected(_))
    ));

    let wrong_root = signed(
        &other,
        anchor.root,
        GroupAction::Add {
            member: individual(&other),
            access: Access::manage(),
        },
        vec![create_id],
        9,
    );
    assert!(auth.accept(&wrong_root).is_err());

    let mut tampered = signed(
        &admin,
        anchor.root,
        GroupAction::Add {
            member: individual(&other),
            access: Access::read(),
        },
        vec![create_id],
        10,
    );
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(matches!(auth.accept(&tampered), Err(AuthError::Header(_))));
    assert_eq!(auth.headers().len(), 1);
    assert_eq!(auth.nodes().len(), 1);
}

#[test]
fn restored_root_uses_new_author_log_without_old_device_tail() {
    let root = SigningKey::generate();
    let restored_root = SigningKey::from_bytes(root.as_bytes());
    let admin = SigningKey::generate();
    let member = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();

    let first_root_action = signed(
        &root,
        anchor.root,
        GroupAction::Add {
            member: individual(&member),
            access: Access::read(),
        },
        vec![create_id],
        11,
    );
    let first_id = auth.accept(&first_root_action).unwrap();
    let restored_action = signed(
        &restored_root,
        anchor.root,
        GroupAction::Promote {
            member: individual(&member),
            access: Access::manage(),
        },
        vec![first_id],
        12,
    );
    auth.accept(&restored_action).unwrap();
    assert!(
        auth.nodes()
            .iter()
            .any(|(id, access)| { *id == member.verifying_key() && *access == Access::manage() })
    );

    let colliding_log = signed(
        &restored_root,
        anchor.root,
        GroupAction::Demote {
            member: individual(&member),
            access: Access::read(),
        },
        vec![Hash::digest(&restored_action)],
        11,
    );
    assert!(matches!(
        auth.accept(&colliding_log),
        Err(AuthError::InvalidLog)
    ));
    assert_eq!(
        ManagementAuth::replay(anchor, auth.headers())
            .unwrap()
            .nodes()
            .len(),
        2
    );
}

#[test]
fn same_author_log_requires_contiguous_sequence_and_backlink() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let member = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();

    let extension = |action, dependencies| ManagementExtension {
        version: 1,
        log_id: [1; 16],
        group: GroupsExtensionArgs {
            group_id: anchor.root,
            action,
            dependencies,
        },
    };
    let add = Header::<ManagementExtension>::builder()
        .chain(1, create_id)
        .build(
            &root,
            extension(
                GroupAction::Add {
                    member: individual(&member),
                    access: Access::read(),
                },
                vec![create_id],
            ),
        )
        .encode();
    let add_id = auth.accept(&add).unwrap();

    let wrong_backlink = Header::<ManagementExtension>::builder()
        .chain(2, create_id)
        .build(
            &root,
            extension(
                GroupAction::Promote {
                    member: individual(&member),
                    access: Access::manage(),
                },
                vec![add_id],
            ),
        )
        .encode();
    assert!(matches!(
        auth.accept(&wrong_backlink),
        Err(AuthError::InvalidLog)
    ));

    let promote = Header::<ManagementExtension>::builder()
        .chain(2, add_id)
        .build(
            &root,
            extension(
                GroupAction::Promote {
                    member: individual(&member),
                    access: Access::manage(),
                },
                vec![add_id],
            ),
        )
        .encode();
    auth.accept(&promote).unwrap();
    assert!(
        auth.nodes()
            .iter()
            .any(|(id, access)| { *id == member.verifying_key() && *access == Access::manage() })
    );
}

#[test]
fn same_log_backlink_must_also_be_a_causal_predecessor() {
    let root = SigningKey::generate();
    let admin = SigningKey::generate();
    let bridge_member = SigningKey::generate();
    let (anchor, create) = genesis(&root, &admin);
    let mut auth = ManagementAuth::new(anchor);
    let create_id = auth.accept(&create).unwrap();

    let extension = |dependencies| ManagementExtension {
        version: 1,
        log_id: [1; 16],
        group: GroupsExtensionArgs {
            group_id: anchor.root,
            action: GroupAction::Promote {
                member: individual(&admin),
                access: Access::manage(),
            },
            dependencies,
        },
    };
    let demote = Header::<ManagementExtension>::builder()
        .chain(1, create_id)
        .build(
            &root,
            ManagementExtension {
                version: 1,
                log_id: [1; 16],
                group: GroupsExtensionArgs {
                    group_id: anchor.root,
                    action: GroupAction::Demote {
                        member: individual(&admin),
                        access: Access::read(),
                    },
                    dependencies: vec![create_id],
                },
            },
        )
        .encode();
    let demote_id = auth.accept(&demote).unwrap();

    let stale_promotion = Header::<ManagementExtension>::builder()
        .chain(2, demote_id)
        .build(&root, extension(vec![create_id]))
        .encode();
    assert!(matches!(
        auth.accept(&stale_promotion),
        Err(AuthError::InvalidCausality)
    ));
    assert!(
        auth.nodes()
            .iter()
            .any(|(id, access)| { *id == admin.verifying_key() && *access == Access::read() })
    );

    let bridge = signed(
        &root,
        anchor.root,
        GroupAction::Add {
            member: individual(&bridge_member),
            access: Access::read(),
        },
        vec![demote_id],
        14,
    );
    let bridge_id = auth.accept(&bridge).unwrap();
    let valid_promotion = Header::<ManagementExtension>::builder()
        .chain(2, demote_id)
        .build(&root, extension(vec![bridge_id]))
        .encode();
    auth.accept(&valid_promotion).unwrap();
    assert!(
        auth.nodes()
            .iter()
            .any(|(id, access)| { *id == admin.verifying_key() && *access == Access::manage() })
    );
}
