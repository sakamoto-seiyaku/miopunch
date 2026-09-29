//! Durable native node entry for creating and reopening one private network.
//!
//! A completed data directory contains four separate responsibilities: the
//! device signing key, the trusted genesis credential, the public network
//! anchor, and the admission secret. The complete signed Create and its
//! authorization view are committed by `ManagementStore` before publication.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use thiserror::Error;

use crate::admission_gate::AdmissionGate;
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::genesis_recovery::GenesisCredential;
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};

const NODE_KEY: &str = "node.key";
const GENESIS_CREDENTIAL: &str = "genesis.credential";
const ANCHOR: &str = "network.anchor";
const ADMISSION_SECRET: &str = "admission.secret";
const MANAGEMENT_DB: &str = "management.sqlite";

struct SavedAnchor([u8; 64]);

impl SavedAnchor {
    fn encode(anchor: NetworkAnchor) -> Self {
        let mut bytes = [0; 64];
        bytes[..32].copy_from_slice(anchor.root.as_bytes());
        bytes[32..].copy_from_slice(anchor.genesis.as_bytes());
        Self(bytes)
    }

    fn decode(self) -> Result<NetworkAnchor, NodeError> {
        Ok(NetworkAnchor {
            root: VerifyingKey::from_bytes(self.0[..32].try_into().unwrap())
                .map_err(|_| NodeError::InvalidData)?,
            genesis: Hash::from_bytes(self.0[32..].try_into().unwrap()),
        })
    }
}

struct SavedGenesisCredential([u8; 64]);

impl SavedGenesisCredential {
    fn encode(root: &SigningKey, genesis: Hash) -> Self {
        let mut bytes = [0; 64];
        bytes[..32].copy_from_slice(root.as_bytes());
        bytes[32..].copy_from_slice(genesis.as_bytes());
        Self(bytes)
    }

    fn decode(self) -> GenesisCredential {
        GenesisCredential::from_saved(
            self.0[..32].try_into().unwrap(),
            Hash::from_bytes(self.0[32..].try_into().unwrap()),
        )
    }
}

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("node data directory already exists")]
    AlreadyExists,
    #[error("node data is incomplete or invalid")]
    InvalidData,
    #[error("genesis credential does not match the pinned network anchor")]
    InvalidCredential,
    #[error("signed management record has not been applied")]
    NotApplied,
    #[error("atomic network directory publication is unavailable on this Unix target")]
    UnsupportedAtomicPublication,
    #[error("node file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("management state failed: {0}")]
    Store(#[from] StoreError),
    #[error("endpoint failed: {0}")]
    Endpoint(#[from] iroh::endpoint::BindError),
}

/// Public, secret-free view obtained only after replaying the signed history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeStatus {
    pub node_id: VerifyingKey,
    pub anchor: NetworkAnchor,
    pub members: Vec<(VerifyingKey, Access)>,
    pub applied_count: i64,
}

/// One verified local network. No secret-bearing type implements `Debug` here.
pub struct NativeNetwork {
    node_key: SigningKey,
    anchor: NetworkAnchor,
    store: ManagementStore,
    gate: AdmissionGate,
}

impl NativeNetwork {
    /// Create a fresh network and device in a new directory. Publishing the
    /// prepared directory is the final step, so readers never see half a Create.
    pub async fn create(path: impl AsRef<Path>) -> Result<Self, NodeError> {
        let path = path.as_ref();
        ensure_absent(path)?;
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        fs::create_dir_all(parent)?;
        let staging = parent.join(format!(".miopunch-create-{:032x}", rand::random::<u128>()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&staging)?;
        let prepared = Self::prepare(&staging).await;
        if let Err(error) = prepared {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let Err(error) = ensure_absent(path) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if let Err(error) = publish_new_directory(&staging, path, parent) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        File::open(parent)?.sync_all()?;
        Self::open(path).await
    }

    async fn prepare(path: &Path) -> Result<(), NodeError> {
        let node_key = SigningKey::generate();
        let root = SigningKey::generate();
        let admission_secret = AdmissionSecret::generate();
        let header = Header::<ManagementExtension>::builder()
            .build(
                &root,
                ManagementExtension {
                    version: 1,
                    log_id: rand::random(),
                    group: GroupsExtensionArgs {
                        group_id: root.verifying_key(),
                        action: GroupAction::Create {
                            initial_members: vec![
                                (
                                    GroupMember::Individual(root.verifying_key()),
                                    Access::manage(),
                                ),
                                (
                                    GroupMember::Individual(node_key.verifying_key()),
                                    Access::manage(),
                                ),
                            ],
                        },
                        dependencies: vec![],
                    },
                },
            )
            .encode();
        let anchor = NetworkAnchor {
            root: root.verifying_key(),
            genesis: Hash::digest(&header),
        };
        write_private(&path.join(NODE_KEY), node_key.as_bytes())?;
        write_private(
            &path.join(GENESIS_CREDENTIAL),
            &SavedGenesisCredential::encode(&root, anchor.genesis).0,
        )?;
        write_private(&path.join(ANCHOR), &SavedAnchor::encode(anchor).0)?;
        write_private(&path.join(ADMISSION_SECRET), admission_secret.saved_bytes())?;
        let mut store = ManagementStore::open(path.join(MANAGEMENT_DB), anchor).await?;
        let create = ManagementRecord::new(header, None);
        store.ingest(&create).await?;
        if store.apply_ready().await? != 1
            || store.nodes() != vec![(node_key.verifying_key(), Access::manage())]
        {
            return Err(NodeError::InvalidData);
        }
        store
            .set_current_admission(
                &AdmissionScope::new(*anchor.root.as_bytes(), 0),
                &admission_secret,
            )
            .await?;
        store.close().await;
        File::open(path)?.sync_all()?;
        Ok(())
    }

    /// Reopen only a fully committed network. Signed history must contain the
    /// exact Create pinned by both the public anchor and private credential.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, NodeError> {
        let path = path.as_ref();
        let node_key = SigningKey::from_bytes(&read_array::<32>(&path.join(NODE_KEY))?);
        let anchor = SavedAnchor(read_array::<64>(&path.join(ANCHOR))?).decode()?;
        let genesis_credential =
            SavedGenesisCredential(read_array::<64>(&path.join(GENESIS_CREDENTIAL))?).decode();
        if genesis_credential.anchor() != anchor {
            return Err(NodeError::InvalidCredential);
        }
        let admission_secret =
            AdmissionSecret::from_saved(read_array::<32>(&path.join(ADMISSION_SECRET))?);
        if !path.join(MANAGEMENT_DB).is_file() {
            return Err(NodeError::InvalidData);
        }
        let store = ManagementStore::open(path.join(MANAGEMENT_DB), anchor).await?;
        if store.get_record(anchor.genesis).await?.is_none() {
            return Err(NodeError::InvalidData);
        }
        let scope = AdmissionScope::new(*anchor.root.as_bytes(), 0);
        if !store
            .current_admission_matches(&scope, &admission_secret.fingerprint(&scope))
            .await?
        {
            return Err(NodeError::InvalidData);
        }
        let node_id = SecretKey::from_bytes(node_key.as_bytes()).public();
        let gate = AdmissionGate::new(node_id, scope, Some(admission_secret));
        gate.register_p2panda_defaults(*anchor.root.as_bytes());
        gate.update_authorization(
            store.nodes().into_iter().map(|(id, _)| *id.as_bytes()),
            store.removed_nodes().into_iter().map(|id| *id.as_bytes()),
        );
        Ok(Self {
            node_key,
            anchor,
            store,
            gate,
        })
    }

    pub fn status(&self) -> NodeStatus {
        let mut members = self.store.nodes();
        members.sort_by_key(|(id, _)| *id.as_bytes());
        NodeStatus {
            node_id: self.node_key.verifying_key(),
            anchor: self.anchor,
            members,
            applied_count: self.store.applied_count(),
        }
    }

    /// Bind only after loading the durable device key; the endpoint can never
    /// silently generate a new identity on process restart.
    pub async fn bind_endpoint(&self, bind_ip: Ipv4Addr) -> Result<Endpoint, NodeError> {
        Ok(self
            .gate
            .configure(Endpoint::builder(presets::Minimal))
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .bind_addr(SocketAddr::from((bind_ip, 0)))
            .map_err(|_| NodeError::InvalidData)?
            .bind()
            .await?)
    }

    /// The same policy handle can be installed through p2panda's endpoint
    /// builder hook and used to classify any additional protocol ALPN.
    pub fn admission_gate(&self) -> AdmissionGate {
        self.gate.clone()
    }

    /// Ingest and apply a complete signed management record through the same
    /// verifier used by replay. Rejected records cannot enter the public view.
    pub async fn ingest_management_record(
        &mut self,
        header: &[u8],
        body: Option<&[u8]>,
    ) -> Result<Hash, NodeError> {
        let id = self
            .store
            .ingest(&ManagementRecord::new(header.to_vec(), body))
            .await?;
        self.store.apply_ready().await?;
        if self.store.get_record(id).await?.is_none() {
            return Err(NodeError::NotApplied);
        }
        self.gate.update_authorization(
            self.store.nodes().into_iter().map(|(id, _)| *id.as_bytes()),
            self.store
                .removed_nodes()
                .into_iter()
                .map(|id| *id.as_bytes()),
        );
        Ok(id)
    }

    pub async fn close(self) {
        self.store.close().await;
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), NodeError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_array<const N: usize>(path: &Path) -> Result<[u8; N], NodeError> {
    let bytes = fs::read(path)?;
    bytes.try_into().map_err(|_| NodeError::InvalidData)
}

fn ensure_absent(path: &Path) -> Result<(), NodeError> {
    match path.symlink_metadata() {
        Ok(_) => Err(NodeError::AlreadyExists),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn publish_new_directory(staging: &Path, target: &Path, parent: &Path) -> Result<(), NodeError> {
    use rustix::fs::{RenameFlags, renameat_with};

    let parent_dir = File::open(parent)?;
    let source = staging.file_name().ok_or(NodeError::InvalidData)?;
    let destination = target.file_name().ok_or(NodeError::InvalidData)?;
    renameat_with(
        &parent_dir,
        source,
        &parent_dir,
        destination,
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        if error == rustix::io::Errno::EXIST {
            NodeError::AlreadyExists
        } else {
            NodeError::Io(error.into())
        }
    })
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))
))]
fn publish_new_directory(_staging: &Path, _target: &Path, _parent: &Path) -> Result<(), NodeError> {
    Err(NodeError::UnsupportedAtomicPublication)
}

#[cfg(not(unix))]
fn publish_new_directory(staging: &Path, target: &Path, _parent: &Path) -> Result<(), NodeError> {
    fs::rename(staging, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn publishing_never_replaces_an_existing_empty_directory() {
        let base = tempfile::tempdir().unwrap();
        let staging = base.path().join("staging");
        let target = base.path().join("target");
        std::fs::create_dir(&staging).unwrap();
        std::fs::create_dir(&target).unwrap();
        assert!(matches!(
            super::publish_new_directory(&staging, &target, base.path()),
            Err(super::NodeError::AlreadyExists)
        ));
        assert!(staging.is_dir());
        assert!(target.is_dir());
    }
}
