//! Narrow signed-management-record validation for MIOPUNCH-23.
//!
//! The root key is an authority identity, never a device. An authenticated
//! p2panda header carries a group action and causal dependencies. Every path
//! (local creation, remote receipt, or history replay) enters through `accept`.

use std::collections::{HashMap, HashSet};

use p2panda_auth::group::resolver::StrongRemove;
use p2panda_auth::group::{GroupAction, GroupCrdt, GroupCrdtError, GroupCrdtState, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs, GroupsOperation};
use p2panda_core::{Hash, Header, HeaderError, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

type Resolver = StrongRemove<VerifyingKey, Hash, GroupsOperation, ()>;
type Crdt = GroupCrdt<VerifyingKey, Hash, GroupsOperation, (), Resolver>;
type State = GroupCrdtState<VerifyingKey, Hash, GroupsOperation, ()>;
type CrdtError = GroupCrdtError<VerifyingKey, Hash, GroupsOperation, (), Resolver>;

/// The trusted, immutable identity of one network and its one canonical Create.
/// It must be obtained from a trusted invitation or previously verified state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkAnchor {
    pub root: VerifyingKey,
    pub genesis: Hash,
}

/// Signed p2panda header extension. A fresh log id lets a device restored
/// from the same root key start its own `(author, log_id)` sequence at zero.
/// Log ids are per-author random values; their uniqueness and durable storage
/// are the responsibility of the later storage/lifecycle integration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagementExtension {
    pub version: u8,
    pub log_id: [u8; 16],
    pub group: GroupsExtensionArgs,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid signed p2panda header: {0}")]
    Header(#[from] HeaderError),
    #[error("wrong network, version, or non-empty management payload")]
    InvalidEnvelope,
    #[error("the operation depends on unavailable history")]
    MissingHistory,
    #[error("author log sequence or backlink is invalid")]
    InvalidLog,
    #[error("author log predecessor is absent from the causal dependencies")]
    InvalidCausality,
    #[error("group authorization failed: {0}")]
    Group(#[from] CrdtError),
}

/// Volatile verification graph for the narrow authorization experiment.
/// Durable storage, ingest transactions, and recovery bootstrap are later tickets.
pub struct ManagementAuth {
    anchor: NetworkAnchor,
    state: State,
    log_heads: HashMap<(VerifyingKey, [u8; 16]), (u32, Hash)>,
    headers: Vec<Vec<u8>>,
}

impl ManagementAuth {
    pub fn new(anchor: NetworkAnchor) -> Self {
        Self {
            anchor,
            state: Crdt::init(),
            log_heads: HashMap::new(),
            headers: Vec::new(),
        }
    }

    /// Verify and apply one raw signed header. `Header::decode` verifies the
    /// Ed25519 signature; the action and dependencies are inside that signature.
    /// Failed operations leave this view and its retained history untouched.
    pub fn accept(&mut self, bytes: &[u8]) -> Result<Hash, AuthError> {
        let header = Header::<ManagementExtension>::decode(bytes)?;
        let extension = &header.extensions;
        if extension.version != 1
            || extension.group.group_id != self.anchor.root
            || header.payload_size != 0
        {
            return Err(AuthError::InvalidEnvelope);
        }

        let operation = GroupsOperation {
            id: header.hash(),
            author: header.verifying_key,
            dependencies: extension.group.dependencies.clone(),
            group_id: extension.group.group_id,
            action: extension.group.action.clone(),
        };
        if operation
            .dependencies
            .iter()
            .any(|id| !self.state.inner.operations.contains_key(id))
        {
            return Err(AuthError::MissingHistory);
        }
        let log_key = (header.verifying_key, extension.log_id);
        match self.log_heads.get(&log_key) {
            None if header.seq_num == 0 && header.backlink.is_none() => {}
            Some((seq, hash))
                if seq.checked_add(1) == Some(header.seq_num) && header.backlink == Some(*hash) =>
            {
                if !has_ancestor(*hash, &operation.dependencies, &self.state) {
                    return Err(AuthError::InvalidCausality);
                }
            }
            _ => return Err(AuthError::InvalidLog),
        }

        let anchor = self.anchor;
        let next = Crdt::process_checked(self.state.clone(), &operation, |causal, op| {
            policy(anchor, causal, op)
        })?;
        self.state = next;
        self.log_heads
            .insert(log_key, (header.seq_num, operation.id));
        self.headers.push(bytes.to_vec());
        Ok(operation.id)
    }

    /// Replays raw signed history through precisely the same verifier as receipt.
    pub fn replay(anchor: NetworkAnchor, headers: &[Vec<u8>]) -> Result<Self, AuthError> {
        let mut auth = Self::new(anchor);
        for header in headers {
            auth.accept(header)?;
        }
        Ok(auth)
    }

    pub fn headers(&self) -> &[Vec<u8>] {
        &self.headers
    }

    /// Effective device membership; excludes the root's virtual Manage entry.
    pub fn nodes(&self) -> Vec<(VerifyingKey, Access)> {
        self.state
            .members(self.anchor.root)
            .into_iter()
            .filter(|(id, _)| *id != self.anchor.root)
            .collect()
    }

    pub fn contains_operation(&self, id: Hash) -> bool {
        self.state.inner.operations.contains_key(&id)
    }

    pub fn filtered_operation(&self, id: Hash) -> bool {
        self.state.inner.ignore.contains(&id)
    }
}

fn has_ancestor(target: Hash, dependencies: &[Hash], state: &State) -> bool {
    let mut pending = dependencies.to_vec();
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        if id == target {
            return true;
        }
        if visited.insert(id) {
            if let Some(operation) = state.inner.operations.get(&id) {
                pending.extend(operation.dependencies.iter().copied());
            }
        }
    }
    false
}

fn policy(
    anchor: NetworkAnchor,
    causal: Option<&p2panda_auth::group::GroupMembersState<GroupMember<VerifyingKey>, ()>>,
    op: &GroupsOperation,
) -> bool {
    if let GroupAction::Create { initial_members } = &op.action {
        return causal.is_none()
            && op.id == anchor.genesis
            && op.author == anchor.root
            && op.dependencies.is_empty()
            && initial_members.len() == 2
            && initial_members.iter().any(|(member, access)| {
                *member == GroupMember::Individual(anchor.root) && *access == Access::manage()
            })
            && initial_members.iter().any(|(member, access)| {
                matches!(member, GroupMember::Individual(id) if *id != anchor.root)
                    && *access == Access::manage()
            });
    }

    let Some(causal) = causal else {
        return false;
    };
    let is_root = op.author == anchor.root;
    match &op.action {
        GroupAction::Add { member, access } => {
            let GroupMember::Individual(target) = member else {
                return false;
            };
            *target != anchor.root
                && (access == &Access::read() || (is_root && access == &Access::manage()))
        }
        GroupAction::Remove { member } => {
            let GroupMember::Individual(target) = member else {
                return false;
            };
            *target != anchor.root
                && (is_root
                    || causal
                        .access_levels()
                        .iter()
                        .any(|(id, access)| *id == *member && *access == Access::read()))
        }
        GroupAction::Promote { member, access } => {
            is_root
                && matches!(member, GroupMember::Individual(id) if *id != anchor.root)
                && *access == Access::manage()
        }
        GroupAction::Demote { member, access } => {
            is_root
                && matches!(member, GroupMember::Individual(id) if *id != anchor.root)
                && *access == Access::read()
        }
        GroupAction::Create { .. } => false,
    }
}
