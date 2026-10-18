use p2panda_auth::group::resolver::StrongRemove;
use p2panda_auth::group::{
    CheckedGroupCrdtError, GroupAction, GroupCrdt, GroupCrdtError, GroupMember,
};
use p2panda_auth::{Access, GroupsOperation};
use p2panda_core::{Hash, SigningKey, VerifyingKey};

type Resolver = StrongRemove<VerifyingKey, Hash, GroupsOperation, ()>;
type Crdt = GroupCrdt<VerifyingKey, Hash, GroupsOperation, (), Resolver>;

mod unchanged_original_consumer {
    include!("auth_consumers/original.rs");

    #[test]
    fn old_process_and_exhaustive_match() {
        main();
    }
}

#[test]
fn checked_rejection_and_original_group_error_stay_distinct() {
    let root = SigningKey::generate().verifying_key();
    let create = GroupsOperation {
        id: Hash::digest(b"create"),
        author: root,
        dependencies: vec![],
        group_id: root,
        action: GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(root), Access::manage())],
        },
    };
    let empty = Crdt::init();
    let rejected = Crdt::process_checked(empty.clone(), &create, |previous, _| {
        assert!(previous.is_none());
        false
    });
    assert!(matches!(rejected, Err(CheckedGroupCrdtError::PolicyRejected(id)) if id == create.id));
    assert!(empty.inner.operations.is_empty() && empty.inner.graph.node_count() == 0);

    let accepted = Crdt::process_checked(empty, &create, |_, _| true).unwrap();
    let duplicate = Crdt::process_checked(accepted, &create, |_, _| {
        panic!("duplicate must fail the original validation before policy")
    });
    assert!(matches!(
        duplicate,
        Err(CheckedGroupCrdtError::Group(GroupCrdtError::DuplicateOperation(id, group)))
            if id == create.id && group == root
    ));
}

#[test]
fn checked_policy_observes_causal_predecessor_before_insertion() {
    let root = SigningKey::generate().verifying_key();
    let admin = SigningKey::generate().verifying_key();
    let candidate = SigningKey::generate().verifying_key();
    let create = GroupsOperation {
        id: Hash::digest(b"causal-create"),
        author: root,
        dependencies: vec![],
        group_id: root,
        action: GroupAction::Create {
            initial_members: vec![
                (GroupMember::Individual(root), Access::manage()),
                (GroupMember::Individual(admin), Access::manage()),
            ],
        },
    };
    let state = Crdt::process(Crdt::init(), &create).unwrap();
    let demote = GroupsOperation {
        id: Hash::digest(b"concurrent-demote"),
        author: root,
        dependencies: vec![create.id],
        group_id: root,
        action: GroupAction::Demote {
            member: GroupMember::Individual(admin),
            access: Access::read(),
        },
    };
    let state = Crdt::process(state, &demote).unwrap();
    assert!(state.members(root).contains(&(admin, Access::read())));
    let grant = GroupsOperation {
        id: Hash::digest(b"concurrent-grant"),
        author: admin,
        dependencies: vec![create.id],
        group_id: root,
        action: GroupAction::Add {
            member: GroupMember::Individual(candidate),
            access: Access::read(),
        },
    };
    let mut called = false;
    let result = Crdt::process_checked(state.clone(), &grant, |previous, _| {
        called = true;
        assert!(
            previous
                .unwrap()
                .managers()
                .contains(&GroupMember::Individual(admin))
        );
        false
    });
    assert!(called);
    assert!(matches!(result, Err(CheckedGroupCrdtError::PolicyRejected(id)) if id == grant.id));
    assert!(!state.inner.operations.contains_key(&grant.id));
    assert!(!state.inner.graph.contains_node(grant.id));
    assert_eq!(state.inner.operations.len(), 2);
}
