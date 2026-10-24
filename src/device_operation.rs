//! Action-specific device decisions shared by Native and Browser hosts.
//!
//! A request is a description, never a bearer or an elevation window. A signed
//! approval binds its complete description to exactly one management record.
use std::io;

use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{
    management_action::ManagementAction,
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
};

pub const DEVICE_ALPN: &[u8] = b"miopunch/device-management/1";
/// A result-only scope remains usable after the initiator's own departure.
pub const DEVICE_RESULT_ALPN: &[u8] = b"miopunch/device-result/1";
pub const MAX_OPERATIONS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAction {
    AppointAdministrator,
    DemoteAdministrator,
    RemoveMember,
    Leave,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Member,
    Administrator,
}

impl DeviceRole {
    fn of(auth: &ManagementAuth, target: VerifyingKey) -> Option<Self> {
        auth.nodes().into_iter().find_map(|(id, access)| {
            (id == target).then_some(if access == Access::manage() {
                Self::Administrator
            } else {
                Self::Member
            })
        })
    }
}

/// Everything a custodian confirms. `target_authorization` distinguishes a
/// removed/rejoined or demoted/reappointed target from its earlier episode.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationDescription {
    pub network: NetworkAnchor,
    pub nonce: [u8; 16],
    pub initiator: VerifyingKey,
    pub initiator_authorization: Hash,
    pub action: DeviceAction,
    pub target: VerifyingKey,
    pub target_role: DeviceRole,
    pub target_authorization: Hash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceOperation {
    pub id: Hash,
    pub description: OperationDescription,
}

impl DeviceOperation {
    pub fn propose(
        network: NetworkAnchor,
        auth: &ManagementAuth,
        initiator: VerifyingKey,
        action: DeviceAction,
        target: VerifyingKey,
    ) -> io::Result<Self> {
        let target_role = DeviceRole::of(auth, target).ok_or_else(rejected)?;
        let description = OperationDescription {
            network,
            nonce: rand::random(),
            initiator,
            initiator_authorization: auth
                .authorization_origin_at(initiator, &auth.tips(), false)
                .ok_or_else(rejected)?,
            action,
            target,
            target_role,
            target_authorization: auth
                .authorization_origin_at(
                    target,
                    &auth.tips(),
                    target_role == DeviceRole::Administrator,
                )
                .ok_or_else(rejected)?,
        };
        let operation = Self {
            id: Hash::digest(serde_json::to_vec(&description)?),
            description,
        };
        operation.check_current(network, auth)?;
        Ok(operation)
    }

    pub fn check_description(&self, network: NetworkAnchor) -> io::Result<()> {
        let d = &self.description;
        if d.network != network || self.id != Hash::digest(serde_json::to_vec(d)?) {
            return Err(rejected());
        }
        match d.action {
            DeviceAction::AppointAdministrator if d.target_role != DeviceRole::Member => {
                return Err(rejected());
            }
            DeviceAction::DemoteAdministrator if d.target_role != DeviceRole::Administrator => {
                return Err(rejected());
            }
            DeviceAction::Leave if d.target != d.initiator => return Err(rejected()),
            _ => {}
        }
        Ok(())
    }

    pub fn check_current(&self, network: NetworkAnchor, auth: &ManagementAuth) -> io::Result<()> {
        self.check_description(network)?;
        let d = &self.description;
        if DeviceRole::of(auth, d.target) != Some(d.target_role)
            || auth.authorization_origin_at(d.initiator, &auth.tips(), false)
                != Some(d.initiator_authorization)
            || auth.authorization_origin_at(
                d.target,
                &auth.tips(),
                d.target_role == DeviceRole::Administrator,
            ) != Some(d.target_authorization)
        {
            return Err(rejected());
        }
        Ok(())
    }

    pub fn requires_genesis(&self) -> bool {
        matches!(
            self.description.action,
            DeviceAction::AppointAdministrator | DeviceAction::DemoteAdministrator
        ) || self.description.target_role == DeviceRole::Administrator
    }

    /// Secret-free, stable impact text for a host's confirmation surface.
    pub fn impact(&self) -> &'static str {
        match self.description.action {
            DeviceAction::AppointAdministrator => {
                "grant daily administration; do not copy the genesis credential"
            }
            DeviceAction::DemoteAdministrator => {
                "revoke daily administration; keep ordinary membership and retained root copies"
            }
            DeviceAction::RemoveMember => {
                "end membership when received; retained genesis copies cannot be invalidated"
            }
            DeviceAction::Leave => {
                "end this node's membership; rejoining requires fresh authorization"
            }
        }
    }

    fn management_action(&self) -> ManagementAction {
        let target = self.description.target;
        match self.description.action {
            DeviceAction::AppointAdministrator => ManagementAction::AppointAdministrator(target),
            DeviceAction::DemoteAdministrator => ManagementAction::DemoteAdministrator(target),
            DeviceAction::RemoveMember | DeviceAction::Leave => {
                ManagementAction::RemoveMember(target)
            }
        }
    }
    fn current_access(&self) -> Option<Access> {
        Some(match self.description.target_role {
            DeviceRole::Member => Access::read(),
            DeviceRole::Administrator => Access::manage(),
        })
    }

    pub fn approve(
        &self,
        network: NetworkAnchor,
        auth: &ManagementAuth,
        author: &SigningKey,
    ) -> io::Result<DeviceApproval> {
        self.check_current(network, auth)?;
        if self.requires_genesis() && author.verifying_key() != network.root
            || !self.requires_genesis() && author.verifying_key() != self.description.initiator
            || auth.has_author_log_id(author.verifying_key(), self.description.nonce)
        {
            return Err(rejected());
        }
        let record = Header::<ManagementExtension>::builder()
            .build(
                author,
                ManagementExtension {
                    version: 1,
                    log_id: self.description.nonce,
                    group: GroupsExtensionArgs {
                        group_id: network.root,
                        action: self.management_action().group_action(self.current_access()),
                        dependencies: auth.tips(),
                    },
                },
            )
            .encode();
        let mut check = auth.clone();
        check.accept(&record).map_err(io::Error::other)?;
        let signature = author
            .sign(&approval_statement(self, &record)?)
            .to_bytes()
            .to_vec();
        Ok(DeviceApproval {
            operation: self.clone(),
            record,
            signature,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceApproval {
    pub operation: DeviceOperation,
    pub record: Vec<u8>,
    pub signature: Vec<u8>,
}
impl DeviceApproval {
    pub fn verify(&self, expected: &DeviceOperation, network: NetworkAnchor) -> io::Result<()> {
        expected.check_description(network)?;
        if &self.operation != expected {
            return Err(rejected());
        }
        let header =
            Header::<ManagementExtension>::decode(&self.record).map_err(io::Error::other)?;
        let signature: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| rejected())?;
        if header.extensions.log_id != expected.description.nonce
            || header.extensions.group.group_id != network.root
            || header.extensions.group.action
                != expected
                    .management_action()
                    .group_action(expected.current_access())
            || expected.requires_genesis() && header.verifying_key != network.root
            || !expected.requires_genesis()
                && header.verifying_key != expected.description.initiator
            || !header.verifying_key.verify(
                &approval_statement(expected, &self.record)?,
                &Signature::from_bytes(&signature),
            )
        {
            return Err(rejected());
        }
        Ok(())
    }
    pub fn record_id(&self) -> Hash {
        Hash::digest(&self.record)
    }
    pub fn effect_current(&self, auth: &ManagementAuth) -> bool {
        if !auth.contains_operation(self.record_id()) || auth.filtered_operation(self.record_id()) {
            return false;
        }
        let target = self.operation.description.target;
        match self.operation.description.action {
            DeviceAction::AppointAdministrator => {
                auth.administrator_authorization_valid(self.record_id(), target)
            }
            DeviceAction::DemoteAdministrator => {
                DeviceRole::of(auth, target) == Some(DeviceRole::Member)
            }
            DeviceAction::RemoveMember | DeviceAction::Leave => {
                DeviceRole::of(auth, target).is_none()
            }
        }
    }
}

fn approval_statement(operation: &DeviceOperation, record: &[u8]) -> io::Result<Vec<u8>> {
    let mut statement = b"miopunch/device-operation-approval/1\0".to_vec();
    statement.extend(serde_json::to_vec(&(operation, record))?);
    Ok(statement)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationEntry {
    pub operation: DeviceOperation,
    pub approval: Option<DeviceApproval>,
    #[serde(default)]
    pub receipt_history: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}
impl OperationEntry {
    /// Queries of an existing result are valid for both ordinary and genesis
    /// actions. Only a new genesis decision belongs on the confirm interface.
    pub fn confirmation(&self) -> io::Result<Option<&DeviceApproval>> {
        if self.approval.is_none() && !self.operation.requires_genesis() {
            return Err(rejected());
        }
        Ok(self.approval.as_ref())
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct OperationJournal {
    pub entries: Vec<OperationEntry>,
}
impl OperationJournal {
    pub fn verify(&self, network: NetworkAnchor) -> io::Result<()> {
        if self.entries.len() > MAX_OPERATIONS {
            return Err(rejected());
        }
        let mut ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.operation.check_description(network)?;
            if !ids.insert(entry.operation.id) {
                return Err(rejected());
            }
            if let Some(approval) = &entry.approval {
                approval.verify(&entry.operation, network)?;
                let auth = ManagementAuth::replay_records(network, &entry.receipt_history)
                    .map_err(io::Error::other)?;
                if !auth.contains_operation(approval.record_id()) {
                    return Err(rejected());
                }
            } else if !entry.receipt_history.is_empty() {
                return Err(rejected());
            }
        }
        Ok(())
    }
    pub fn get(&self, id: Hash) -> io::Result<&OperationEntry> {
        self.entries
            .iter()
            .find(|e| e.operation.id == id)
            .ok_or_else(rejected)
    }
    pub fn retain(&mut self, operation: DeviceOperation) -> io::Result<()> {
        if let Ok(existing) = self.get(operation.id) {
            return if existing.operation == operation {
                Ok(())
            } else {
                Err(rejected())
            };
        }
        if self.entries.len() >= MAX_OPERATIONS {
            return Err(rejected());
        }
        self.entries.push(OperationEntry {
            operation,
            approval: None,
            receipt_history: Vec::new(),
        });
        Ok(())
    }
    pub fn approve(
        &mut self,
        approval: DeviceApproval,
        history: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    ) -> io::Result<()> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.operation.id == approval.operation.id)
            .ok_or_else(rejected)?;
        approval.verify(&entry.operation, entry.operation.description.network)?;
        if let Some(existing) = &entry.approval {
            if existing != &approval {
                return Err(rejected());
            }
            return Ok(());
        }
        let auth = ManagementAuth::replay_records(entry.operation.description.network, &history)
            .map_err(io::Error::other)?;
        if !auth.contains_operation(approval.record_id())
            || auth.filtered_operation(approval.record_id())
        {
            return Err(rejected());
        }
        entry.receipt_history = history;
        entry.approval = Some(approval);
        Ok(())
    }
}

fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "device operation is invalid or no longer current",
    )
}
