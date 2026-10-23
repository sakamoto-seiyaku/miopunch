//! Bounded transfer of original signed control records. No admission secret,
//! invitation bearer or business file content is included in this history.
use crate::{
    data_stream::DataStream,
    management_auth::{ManagementAuth, NetworkAnchor},
    service_policy::{MAX_RECORDS, ServiceLedger},
};
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
        if incoming.management.len() > MAX_RECORDS {
            return Err(rejected());
        }
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
        if management.len() > MAX_RECORDS {
            return Err(rejected());
        }
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
        if self.management.len() > MAX_RECORDS || self.services.len() > MAX_RECORDS {
            return Err(rejected());
        }
        let auth =
            ManagementAuth::replay_records(anchor, &self.management).map_err(|_| rejected())?;
        let ledger =
            ServiceLedger::replay(anchor, &auth, &self.services).map_err(|_| rejected())?;
        Ok((auth, ledger))
    }
    pub async fn write(&self, stream: &mut DataStream) -> io::Result<()> {
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(rejected());
        }
        for chunk in bytes.chunks(crate::data_stream::CHUNK_BYTES) {
            stream.write_chunk(chunk).await?;
        }
        stream.write_chunk(&[]).await
    }
    pub async fn read(stream: &mut DataStream) -> io::Result<Self> {
        let mut bytes = Vec::new();
        loop {
            let chunk = stream.read_chunk().await?;
            if chunk.is_empty() {
                break;
            }
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err(rejected());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}
fn rejected() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "invalid service history")
}
