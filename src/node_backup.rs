//! Portable complete-node material and explicit replacement receipts.
//!
//! The backup is secret-bearing: only explicit export/import adapters serialize
//! it. Ordinary status and diagnostics use the separate public package/receipt.
use std::io;

use p2panda_core::{Hash, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{
    client_invitation::{BootstrapContact, ClientInvitation},
    credential_custody::CustodyJournal,
    device_operation::OperationJournal,
    management_auth::NetworkAnchor,
    service_policy::GuestInvitation,
    service_protocol::{ManagementRecords, ServiceBundle},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplacementPackage {
    pub version: u8,
    pub id: Hash,
    pub identity: VerifyingKey,
    pub network: NetworkAnchor,
}
impl ReplacementPackage {
    pub fn new(identity: VerifyingKey, network: NetworkAnchor) -> Self {
        Self {
            version: 1,
            id: Hash::digest(rand::random::<[u8; 32]>()),
            identity,
            network,
        }
    }
    pub fn verify(&self) -> io::Result<()> {
        if self.version != 1 || self.identity == self.network.root {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRuntime {
    #[default]
    Native,
    Browser,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PortableSettings {
    #[serde(default)]
    pub source: SourceRuntime,
    pub native: Option<serde_json::Value>,
    pub browser: Option<serde_json::Value>,
    pub contact: BootstrapContact,
    pub path_selection: crate::path_selection::PathSelectionPolicy,
    pub maintenance: serde_json::Value,
}

// Deliberately no Debug. This contains the same identity and complete root copy
// where held, admission secret, original authorization and service history.
#[derive(Clone, Serialize, Deserialize)]
pub struct PortableNode {
    pub key: [u8; 32],
    pub network: NetworkAnchor,
    pub records: ManagementRecords,
    #[serde(default)]
    pub pending_records: ManagementRecords,
    pub authorization: Option<Hash>,
    pub generation: u64,
    pub secret: Option<[u8; 32]>,
    pub genesis_seed: Option<[u8; 32]>,
    pub recovery_exported: bool,
    pub recovery_saved: bool,
    pub complete_invitation: Option<ClientInvitation>,
    pub services: Vec<Vec<u8>>,
    pub guest: Option<GuestInvitation>,
    pub operations: OperationJournal,
    pub custody: CustodyJournal,
    #[serde(default)]
    pub recovery: Option<crate::management_recovery_wire::RecoveryIntent>,
    #[serde(default)]
    pub recoveries: Vec<crate::management_recovery_wire::RecoveryProof>,
    #[serde(default)]
    pub admission: crate::node_control_backup::AdmissionLedger,
    #[serde(default)]
    pub native_rotation: crate::node_control_backup::NativeRotationState,
    pub settings: PortableSettings,
}
impl PortableNode {
    pub fn identity(&self) -> VerifyingKey {
        SigningKey::from_bytes(&self.key).verifying_key()
    }
    pub fn verify(&self) -> io::Result<()> {
        let (auth, _) = ServiceBundle {
            management: self.records.clone(),
            services: self.services.clone(),
        }
        .verify(self.network)?;
        if self.records.first().map(|(h, _)| Hash::digest(h)) != Some(self.network.genesis)
            || self.generation > i64::MAX as u64
            || self.recovery_saved && !self.recovery_exported
            || self
                .genesis_seed
                .is_some_and(|s| SigningKey::from_bytes(&s).verifying_key() != self.network.root)
        {
            return Err(invalid());
        }
        if self.records.len() + self.pending_records.len() > 4096 {
            return Err(invalid());
        }
        for (header, body) in &self.pending_records {
            crate::management_auth::decode_complete(header, body.as_deref(), self.network)
                .map_err(io::Error::other)?;
        }
        self.operations.verify(self.network)?;
        self.custody.verify(self.network)?;
        self.admission.verify_history(self.identity(), &auth)?;
        self.native_rotation.verify_shape()?;
        if self.recoveries.len() > 4096 {
            return Err(invalid());
        }
        let mut recovery_ids = std::collections::BTreeSet::new();
        for proof in &self.recoveries {
            proof.verify(self.network, &self.records)?;
            if !recovery_ids.insert((proof.target, proof.attempt)) {
                return Err(invalid());
            }
        }
        if let Some(intent) = &self.recovery {
            intent.verify(
                self.identity(),
                &self.recoveries,
                self.genesis_seed.is_some(),
            )?;
        }
        self.settings.contact.validate()?;
        if let Some(invitation) = &self.complete_invitation {
            invitation.encode()?;
            let material = invitation.material_bytes()?;
            if material[..32] != *self.network.root.as_bytes()
                || material[32..64] != *self.network.genesis.as_bytes()
            {
                return Err(invalid());
            }
        }
        if let Some(guest) = &self.guest {
            guest.encode().map_err(io::Error::other)?;
            if guest.anchor != self.network {
                return Err(invalid());
            }
        } else if self.secret.is_none()
            || !self
                .authorization
                .is_some_and(|grant| auth.membership_authorization_valid(grant, self.identity()))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeBackup {
    pub package: ReplacementPackage,
    pub node: PortableNode,
}
impl NodeBackup {
    pub fn verify(&self) -> io::Result<()> {
        self.package.verify()?;
        self.node.verify()?;
        if self.package.identity != self.node.identity()
            || self.package.network != self.node.network
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn encode(&self) -> io::Result<String> {
        self.verify()?;
        let text = serde_json::to_string(self)?;
        if text.len() > 80 * 1024 * 1024 {
            return Err(invalid());
        }
        Ok(text)
    }
    pub fn decode(text: &str) -> io::Result<Self> {
        if text.len() > 80 * 1024 * 1024 {
            return Err(invalid());
        }
        let backup: Self = serde_json::from_str(text)?;
        backup.verify()?;
        Ok(backup)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplacementReceipt {
    pub package: ReplacementPackage,
    pub destination: [u8; 16],
    pub qualified_heads: Vec<Hash>,
    pub signature: Vec<u8>,
}
impl ReplacementReceipt {
    pub fn sign(
        package: ReplacementPackage,
        destination: [u8; 16],
        heads: Vec<Hash>,
        key: &SigningKey,
    ) -> io::Result<Self> {
        package.verify()?;
        if key.verifying_key() != package.identity || heads.is_empty() {
            return Err(invalid());
        }
        let signature = key
            .sign(&receipt_statement(&package, destination, &heads)?)
            .to_bytes()
            .to_vec();
        Ok(Self {
            package,
            destination,
            qualified_heads: heads,
            signature,
        })
    }
    pub fn verify(&self, package: &ReplacementPackage) -> io::Result<()> {
        package.verify()?;
        let bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| invalid())?;
        if &self.package != package
            || self.qualified_heads.is_empty()
            || !package.identity.verify(
                &receipt_statement(package, self.destination, &self.qualified_heads)?,
                &Signature::from_bytes(&bytes),
            )
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoredNode {
    pub package: ReplacementPackage,
    pub destination: [u8; 16],
    pub receipt: Option<ReplacementReceipt>,
}
impl RestoredNode {
    pub fn new(package: ReplacementPackage) -> Self {
        Self {
            package,
            destination: rand::random(),
            receipt: None,
        }
    }
    pub fn verify(&self, identity: VerifyingKey, network: NetworkAnchor) -> io::Result<()> {
        self.package.verify()?;
        if self.package.identity != identity || self.package.network != network {
            return Err(invalid());
        }
        if let Some(receipt) = &self.receipt {
            receipt.verify(&self.package)?;
            if receipt.destination != self.destination {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
fn receipt_statement(
    package: &ReplacementPackage,
    destination: [u8; 16],
    heads: &[Hash],
) -> io::Result<Vec<u8>> {
    let mut bytes = b"miopunch/node-replacement-receipt/1\0".to_vec();
    bytes.extend(serde_json::to_vec(&(package, destination, heads))?);
    Ok(bytes)
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "node backup or replacement receipt is invalid",
    )
}
