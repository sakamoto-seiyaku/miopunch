//! Device management exchanges on admitted member and restricted control paths.
use std::{collections::HashSet, io};

use p2panda_core::{Hash, Header, VerifyingKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    control_protocol::AuthenticatedControl,
    data_stream::DataStream,
    device_operation::{DeviceAction, DeviceApproval, DeviceOperation, OperationEntry},
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
    service_protocol::{ManagementRecords, ServiceBundle, read_guest, write_guest},
};

pub(crate) const DEPARTURE_ALPN: &[u8] = b"miopunch/device-departure/1";
pub(crate) const REMOVAL_ALPN: &[u8] = b"miopunch/device-removal/1";

/// Carries one self-departure and only its signed causal predecessors.
#[derive(Serialize, Deserialize)]
pub(crate) struct DepartureDelivery {
    pub(crate) approval: DeviceApproval,
    dependencies: ManagementRecords,
}

impl DepartureDelivery {
    pub(crate) fn new(entry: &OperationEntry) -> io::Result<Self> {
        let approval = entry.approval.as_ref().ok_or_else(invalid_departure)?;
        Ok(Self {
            dependencies: causal_dependencies(&approval.record, &entry.receipt_history)?,
            approval: approval.clone(),
        })
    }

    // Validate the entire restricted upload before touching durable state.
    // None means this exact verified record is already durable.
    pub(crate) fn verify(
        &self,
        anchor: NetworkAnchor,
        current: &ManagementAuth,
        local: VerifyingKey,
        peer: VerifyingKey,
    ) -> io::Result<Option<ManagementRecords>> {
        let operation = &self.approval.operation;
        if operation.description.action != DeviceAction::Leave
            || operation.description.initiator != peer
            || local == peer
            || !current.nodes().iter().any(|(id, _)| *id == local)
            || self.dependencies.len() >= crate::service_policy::MAX_RECORDS
        {
            return Err(invalid_departure());
        }
        self.approval.verify(operation, anchor)?;
        if causal_dependencies(&self.approval.record, &self.dependencies)?.len()
            != self.dependencies.len()
        {
            return Err(invalid_departure());
        }
        let causal =
            ManagementAuth::replay_records(anchor, &self.dependencies).map_err(io::Error::other)?;
        operation.check_current(anchor, &causal)?;
        let record = self.approval.record_id();
        if current.contains_operation(record) && !current.filtered_operation(record) {
            return Ok(None);
        }
        let mut preview = current.clone();
        for (header, body) in &self.dependencies {
            if !preview.contains_operation(Hash::digest(header)) {
                preview
                    .accept_record(header, body.as_deref())
                    .map_err(io::Error::other)?;
            }
        }
        if preview
            .authorization_origin_at(peer, &preview.tips(), false)
            .is_some_and(|origin| origin != operation.description.initiator_authorization)
        {
            return Err(invalid_departure());
        }
        preview
            .accept(&self.approval.record)
            .map_err(io::Error::other)?;
        if !self.approval.effect_current(&preview)
            || !preview.nodes().iter().any(|(id, _)| *id == local)
        {
            return Err(invalid_departure());
        }
        let mut management = self.dependencies.clone();
        management.push((self.approval.record.clone(), None));
        Ok(Some(management))
    }

    pub(crate) async fn send(&self, connection: &impl AuthenticatedControl) -> io::Result<Hash> {
        let exchange = connection.open_control_exchange().await?;
        write_guest(&exchange, self).await?;
        exchange.finish_send().await.map_err(io::Error::other)?;
        let receipt: Hash = read_guest(&exchange).await?;
        if receipt != self.approval.record_id() {
            return Err(invalid_departure());
        }
        Ok(receipt)
    }
}

fn causal_dependencies(decision: &[u8], records: &ManagementRecords) -> io::Result<ManagementRecords> {
    let decision = Header::<ManagementExtension>::decode(decision).map_err(io::Error::other)?;
    let mut needed: HashSet<_> = decision.extensions.group.dependencies.into_iter().collect();
    let mut dependencies = Vec::new();
    for (bytes, body) in records.iter().rev() {
        if needed.remove(&Hash::digest(bytes)) {
            let header = Header::<ManagementExtension>::decode(bytes).map_err(io::Error::other)?;
            needed.extend(header.extensions.group.dependencies);
            dependencies.push((bytes.clone(), body.clone()));
        }
    }
    if !needed.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "missing decision dependencies"));
    }
    dependencies.reverse();
    Ok(dependencies)
}

/// The requested membership grant belongs to the authenticated requester.
/// The reply contains exactly one decision and its signed causal predecessors.
#[derive(Serialize, Deserialize)]
pub(crate) struct RemovalNotice {
    decision: Vec<u8>,
    dependencies: ManagementRecords,
}

impl RemovalNotice {
    pub(crate) fn new(decision: Hash, records: &ManagementRecords) -> io::Result<Self> {
        let (decision, _) = records.iter()
            .find(|(header, _)| Hash::digest(header) == decision)
            .ok_or_else(invalid_removal)?;
        Ok(Self {
            decision: decision.clone(),
            dependencies: causal_dependencies(decision, records)?,
        })
    }

    pub(crate) fn verify(
        mut self,
        current: &ManagementAuth,
        grant: Hash,
        local: VerifyingKey,
        holder: VerifyingKey,
    ) -> io::Result<ManagementRecords> {
        if self.dependencies.len() >= crate::service_policy::MAX_RECORDS
            || !current.membership_authorization_valid(grant, local)
            || current.known_removed(holder)
            || causal_dependencies(&self.decision, &self.dependencies)?.len() != self.dependencies.len()
        {
            return Err(invalid_removal());
        }
        let mut preview = current.clone();
        for (header, body) in &self.dependencies {
            if !preview.contains_operation(Hash::digest(header)) {
                preview.accept_record(header, body.as_deref()).map_err(io::Error::other)?;
            }
        }
        let decision = preview.accept(&self.decision).map_err(io::Error::other)?;
        if !preview.removal_applies_to(decision, grant, local)
            || !preview.known_removed(local)
            || !preview.nodes().iter().any(|(id, _)| *id == holder)
        {
            return Err(invalid_removal());
        }
        self.dependencies.push((self.decision, None));
        Ok(self.dependencies)
    }

    pub(crate) async fn request(
        connection: &impl AuthenticatedControl,
        grant: Hash,
    ) -> io::Result<Option<Self>> {
        let exchange = connection.open_control_exchange().await?;
        write_guest(&exchange, &grant).await?;
        exchange.finish_send().await.map_err(io::Error::other)?;
        read_guest(&exchange).await
    }
}

fn invalid_removal() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "invalid membership removal notice")
}

fn invalid_departure() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "invalid departure delivery",
    )
}

#[derive(Serialize, Deserialize)]
pub enum DeviceRequest {
    Confirm { operation: DeviceOperation },
}

#[derive(Serialize, Deserialize)]
pub struct DeviceReply {
    pub approval: Option<DeviceApproval>,
}

#[derive(Serialize, Deserialize)]
pub struct ResultRequest {
    pub operation: p2panda_core::Hash,
}
#[derive(Serialize, Deserialize)]
pub struct ResultReply {
    pub approval: Option<DeviceApproval>,
    pub history: ServiceBundle,
}

pub async fn write_message(stream: &mut DataStream, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 64 * 1024 {
        return Err(io::Error::other("device message too large"));
    }
    stream.write_chunk(&bytes).await
}

pub async fn read_message<T: DeserializeOwned>(stream: &mut DataStream) -> io::Result<T> {
    Ok(serde_json::from_slice(&stream.read_chunk().await?)?)
}

/// The accompanying signed history has its own existing 16MiB bound.
pub async fn write_exchange(
    stream: &mut DataStream,
    value: &impl Serialize,
    history: &ServiceBundle,
) -> io::Result<()> {
    write_message(stream, value).await?;
    history.write(stream).await
}
