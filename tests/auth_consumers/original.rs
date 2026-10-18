use p2panda_auth::group::resolver::StrongRemove;
use p2panda_auth::group::{GroupAction, GroupCrdt, GroupCrdtError, GroupCrdtState, GroupMember};
use p2panda_auth::{Access, GroupsOperation};
use p2panda_core::{Hash, SigningKey, VerifyingKey};

type Resolver = StrongRemove<VerifyingKey, Hash, GroupsOperation, ()>;
type Crdt = GroupCrdt<VerifyingKey, Hash, GroupsOperation, (), Resolver>;
type State = GroupCrdtState<VerifyingKey, Hash, GroupsOperation, ()>;
type Error = GroupCrdtError<VerifyingKey, Hash, GroupsOperation, (), Resolver>;

fn classify(error: Error) -> &'static str {
    match error {
        Error::Inner(_) => "inner",
        Error::DuplicateOperation(_, _) => "duplicate",
        Error::GroupCycle(_, _, _) => "cycle",
        Error::StateChangeError(_, _) => "state-change",
        Error::ManagerGroupsNotAllowed(_) => "manager-group",
        Error::Resolver(_) => "resolver",
    }
}

fn main() {
    let old_process: fn(State, &GroupsOperation) -> Result<State, Error> = Crdt::process;
    let root = SigningKey::from_bytes(&[1; 32]).verifying_key();
    let operation = GroupsOperation {
        id: Hash::digest(b"unchanged ordinary auth consumer"),
        author: root,
        dependencies: vec![],
        group_id: root,
        action: GroupAction::Create {
            initial_members: vec![(GroupMember::Individual(root), Access::manage())],
        },
    };
    let state = old_process(Crdt::init(), &operation).expect("original process succeeds");
    assert_eq!(state.inner.operations.len(), 1);
    let duplicate = old_process(state, &operation).expect_err("duplicate fails");
    assert_eq!(classify(duplicate), "duplicate");
    println!("unchanged original process and exhaustive error match passed");
}
