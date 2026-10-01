//! Durable native node entry for creating and reopening one private network.
//!
//! A creator saves its device key, trusted genesis credential, public network
//! anchor and admission secret. A joining device saves its own key and trusted
//! invitation, then independently persists verified membership and the current
//! secret. Ordinary joined devices never acquire the genesis private key.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use iroh::endpoint::{Builder, presets};
use iroh::{Endpoint, SecretKey};
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use thiserror::Error;

use crate::admission_gate::AdmissionGate;
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::genesis_recovery::GenesisCredential;
use crate::management_auth::{ManagementExtension, NetworkAnchor};
use crate::management_recovery::RecoveryBinding;
use crate::management_store::{ManagementRecord, ManagementStore, StoreError};
use crate::manual_admission::{JoinBinding, ManualInvitation};
use crate::nat_connection::{NatConnector, UdpPolicy, UdpUpgrade};

const NODE_KEY: &str = "node.key";
const GENESIS_CREDENTIAL: &str = "genesis.credential";
const ANCHOR: &str = "network.anchor";
const ADMISSION_SECRET: &str = "admission.secret";
const MANAGEMENT_DB: &str = "management.sqlite";
const JOIN_BINDING: &str = "join.invitation";
const ADMISSION_MATERIAL: &str = "admission.material";
const RECOVERY_BINDING: &str = "recovery.binding";

enum Initialization<'a> {
    Create,
    Join(ManualInvitation),
    Recover(&'a GenesisCredential, iroh_base::EndpointId),
}

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
    pub(crate) node_key: SigningKey,
    pub(crate) anchor: NetworkAnchor,
    pub(crate) store: ManagementStore,
    pub(crate) gate: AdmissionGate,
    pub(crate) scope: AdmissionScope,
    pub(crate) secret: Option<AdmissionSecret>,
    pub(crate) join: Option<JoinBinding>,
    pub(crate) recovery: Option<RecoveryBinding>,
    pub(crate) management_instance: [u8; 16],
    directory: PathBuf,
    udp: NatConnector,
}

impl NativeNetwork {
    /// Create a fresh network and device in a new directory. Publishing the
    /// prepared directory is the final step, so readers never see half a Create.
    pub async fn create(path: impl AsRef<Path>) -> Result<Self, NodeError> {
        Self::initialize(path.as_ref(), Initialization::Create).await
    }

    /// Initialize an independently identified applicant. The trusted invitation
    /// pins its network and contact, but grants no membership or shared secret.
    pub async fn join(
        path: impl AsRef<Path>,
        invitation: ManualInvitation,
    ) -> Result<Self, NodeError> {
        Self::initialize(path.as_ref(), Initialization::Join(invitation)).await
    }

    /// Prepare a new independent device using the trusted original root-key
    /// and canonical Create backup. This stores no history or admission secret
    /// and grants no access. `restore_management` proves root authority to the
    /// chosen contact over the restricted TLS session; no approval is needed.
    pub async fn recover(
        path: impl AsRef<Path>,
        credential: &GenesisCredential,
        contact: iroh_base::EndpointId,
    ) -> Result<Self, NodeError> {
        Self::initialize(path.as_ref(), Initialization::Recover(credential, contact)).await
    }

    async fn initialize(
        path: &Path,
        initialization: Initialization<'_>,
    ) -> Result<Self, NodeError> {
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
        let prepared = match initialization {
            Initialization::Create => Self::prepare(&staging).await,
            Initialization::Join(invitation) => Self::prepare_join(&staging, invitation).await,
            Initialization::Recover(credential, contact) => {
                Self::prepare_recovery(&staging, credential, contact).await
            }
        };
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

    async fn prepare_recovery(
        path: &Path,
        credential: &GenesisCredential,
        contact: iroh_base::EndpointId,
    ) -> Result<(), NodeError> {
        let key = SigningKey::generate();
        let anchor = credential.anchor();
        let binding = RecoveryBinding {
            contact,
            attempt: rand::random(),
        };
        write_private(&path.join(NODE_KEY), key.as_bytes())?;
        write_private(&path.join(ANCHOR), &SavedAnchor::encode(anchor).0)?;
        write_private(
            &path.join(GENESIS_CREDENTIAL),
            &SavedGenesisCredential::encode(&credential.root, anchor.genesis).0,
        )?;
        write_private(&path.join(RECOVERY_BINDING), &binding.encode())?;
        let store = ManagementStore::open(path.join(MANAGEMENT_DB), anchor).await?;
        store.close().await;
        File::open(path)?.sync_all()?;
        Ok(())
    }

    async fn prepare_join(path: &Path, invitation: ManualInvitation) -> Result<(), NodeError> {
        let key = SigningKey::generate();
        let binding = JoinBinding {
            invitation,
            request_id: rand::random(),
        };
        write_private(&path.join(NODE_KEY), key.as_bytes())?;
        write_private(
            &path.join(ANCHOR),
            &SavedAnchor::encode(invitation.anchor).0,
        )?;
        write_private(&path.join(JOIN_BINDING), &binding.encode())?;
        let store = ManagementStore::open(path.join(MANAGEMENT_DB), invitation.anchor).await?;
        store.close().await;
        File::open(path)?.sync_all()?;
        Ok(())
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

    /// Replay a created network or an applicant's durable progress. Creators
    /// pin the Create with their private recovery credential; applicants use
    /// their trusted invitation and do not become successful members early.
    /// Capture a coherent, trusted local backup after stopping this node.
    /// Callers must close their bound endpoints and avoid concurrently opening
    /// the same data directory. This consumes the node and closes SQLite before
    /// copying its checkpointed main database and private durable materials.
    pub async fn create_backup(self, target: impl AsRef<Path>) -> Result<(), NodeError> {
        let source = self.directory.clone();
        self.store.checkpoint_backup().await?;
        self.close().await;
        copy_node_backup(&source, target.as_ref(), false).await
    }

    /// Restore into a new directory, never over a running node. The trusted
    /// backup may predate consumption, so every restored code is stopped before
    /// atomic publication. This explicit recovery event supplies the boundary;
    /// no flag inside a rolled-back database proves counter continuity. Fresh
    /// codes can be issued normally. Undisclosed whole-disk rollback detection
    /// is intentionally not claimed.
    pub async fn restore_backup(
        backup: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<Self, NodeError> {
        if fs::read(backup.as_ref().join("backup.version"))? != b"miopunch node backup 1\n" {
            return Err(NodeError::InvalidData);
        }
        copy_node_backup(backup.as_ref(), target.as_ref(), true).await?;
        Self::open(target).await
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<Self, NodeError> {
        let path = path.as_ref();
        let node_key = SigningKey::from_bytes(&read_array::<32>(&path.join(NODE_KEY))?);
        let anchor = SavedAnchor(read_array::<64>(&path.join(ANCHOR))?).decode()?;
        let join = if path.join(JOIN_BINDING).exists() {
            let binding = JoinBinding::decode(&read_array::<112>(&path.join(JOIN_BINDING))?)
                .map_err(|_| NodeError::InvalidData)?;
            if binding.invitation.anchor != anchor {
                return Err(NodeError::InvalidData);
            }
            Some(binding)
        } else {
            None
        };
        let recovery = if path.join(RECOVERY_BINDING).exists() {
            Some(RecoveryBinding::decode(&read_array::<48>(
                &path.join(RECOVERY_BINDING),
            )?)?)
        } else {
            None
        };
        if recovery.is_some() && !path.join(GENESIS_CREDENTIAL).exists() {
            return Err(NodeError::InvalidData);
        }
        if path.join(GENESIS_CREDENTIAL).exists() {
            let credential =
                SavedGenesisCredential(read_array::<64>(&path.join(GENESIS_CREDENTIAL))?).decode();
            if credential.anchor() != anchor {
                return Err(NodeError::InvalidCredential);
            }
        } else if join.is_none() {
            return Err(NodeError::InvalidData);
        }
        if !path.join(MANAGEMENT_DB).is_file() {
            return Err(NodeError::InvalidData);
        }
        let mut store = ManagementStore::open(path.join(MANAGEMENT_DB), anchor).await?;
        // Approval's Add/result can have committed before process exit without
        // its materialized view. Recover ready records before exposing the
        // reopened node's authority or making admission decisions.
        store.apply_ready().await?;
        if path.join(GENESIS_CREDENTIAL).exists()
            && recovery.is_none()
            && store.get_record(anchor.genesis).await?.is_none()
        {
            return Err(NodeError::InvalidData);
        }
        let (scope, mut secret) = if path.join(ADMISSION_MATERIAL).exists() {
            let bytes = read_array::<40>(&path.join(ADMISSION_MATERIAL))?;
            (
                AdmissionScope::new(
                    *anchor.root.as_bytes(),
                    u64::from_be_bytes(bytes[..8].try_into().unwrap()),
                ),
                Some(AdmissionSecret::from_saved(bytes[8..].try_into().unwrap())),
            )
        } else if path.join(ADMISSION_SECRET).exists() {
            (
                AdmissionScope::new(*anchor.root.as_bytes(), 0),
                Some(AdmissionSecret::from_saved(read_array::<32>(
                    &path.join(ADMISSION_SECRET),
                )?)),
            )
        } else if join.is_some() || recovery.is_some() {
            (AdmissionScope::new(*anchor.root.as_bytes(), 0), None)
        } else {
            return Err(NodeError::InvalidData);
        };
        if let Some(material) = &secret {
            if !store
                .current_admission_matches(&scope, &material.fingerprint(&scope))
                .await?
            {
                if join.is_none() && recovery.is_none() {
                    return Err(NodeError::InvalidData);
                }
                // A crash between private-file fsync and database commit is
                // still pending. A retry must pin the credential before use.
                secret = None;
            }
        }
        let node_id = SecretKey::from_bytes(node_key.as_bytes()).public();
        let gate = AdmissionGate::new(node_id, scope.clone(), secret.clone());
        gate.register_p2panda_defaults(*anchor.root.as_bytes());
        gate.update_authorization(
            store.nodes().into_iter().map(|(id, _)| *id.as_bytes()),
            store.removed_nodes().into_iter().map(|id| *id.as_bytes()),
        );
        let mut node = Self {
            node_key,
            anchor,
            store,
            gate,
            scope,
            secret,
            join,
            recovery,
            management_instance: rand::random(),
            directory: path.to_path_buf(),
            udp: NatConnector::new(None, UdpPolicy::default()),
        };
        if node.join.is_some() {
            if let Some(binding) = node.recovery {
                if !node.has_completed_root_recovery(binding).await? {
                    return Err(NodeError::InvalidData);
                }
                // A single fsynced join-binding publication selects the new
                // active lifecycle. Keep the old signed recovery history and
                // binding as inert audit material, without two active flows.
                node.recovery = None;
            }
        }
        Ok(node)
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
        self.bind_endpoint_with(
            Endpoint::builder(presets::Minimal)
                .alpns(vec![
                    crate::admission_gate::ADMISSION_REQUEST_ALPN.to_vec(),
                    crate::management_recovery::MANAGEMENT_RECOVERY_ALPN.to_vec(),
                ])
                .bind_addr(SocketAddr::from((bind_ip, 0)))
                .map_err(|_| NodeError::InvalidData)?,
        )
        .await
    }

    /// Configure allowed resources/carriers and advertised protocols through
    /// the selected stack. The node installs its durable identity and shared
    /// gate last, so carrier configuration cannot substitute either boundary.
    pub async fn bind_endpoint_with(&self, builder: Builder) -> Result<Endpoint, NodeError> {
        Ok(self
            .gate
            .configure(builder)
            .secret_key(SecretKey::from_bytes(self.node_key.as_bytes()))
            .bind()
            .await?)
    }

    /// The same policy handle can be installed through p2panda's endpoint
    /// builder hook and used to classify any additional protocol ALPN.
    pub fn admission_gate(&self) -> AdmissionGate {
        self.gate.clone()
    }

    /// Supply the node's explicitly configured observation resources and UDP
    /// maintenance policy when reopening. The connector is retained by the
    /// node across connection attempts rather than recreated after failures.
    pub async fn open_with_udp_resources(
        path: impl AsRef<Path>,
        observers: Option<iroh::RelayMap>,
        policy: UdpPolicy,
    ) -> Result<Self, NodeError> {
        let mut node = Self::open(path).await?;
        node.udp = NatConnector::new(observers, policy);
        Ok(node)
    }

    pub fn udp_connector(&self) -> &NatConnector {
        &self.udp
    }

    /// Upgrade an admitted control session using direct candidates first, then
    /// a bounded sequence of complete automatically selected old UDP plans.
    pub async fn upgrade_udp(
        &self,
        control: crate::control_session::ControlSession,
    ) -> Result<UdpUpgrade, crate::control_session::CommitError> {
        self.udp.upgrade(control, &self.gate).await
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
        self.refresh_authorization();
        Ok(id)
    }

    pub(crate) fn refresh_authorization(&self) {
        self.gate.update_authorization(
            self.store.nodes().into_iter().map(|(id, _)| *id.as_bytes()),
            self.store
                .removed_nodes()
                .into_iter()
                .map(|id| *id.as_bytes()),
        );
    }

    /// Start a fresh approval attempt after effective departure/removal while
    /// preserving this device key, network anchor and all verified history.
    /// An old request keeps its old invalidated result; it is never repurposed.
    pub async fn begin_rejoin(&mut self, invitation: ManualInvitation) -> Result<(), NodeError> {
        self.store.apply_ready().await?;
        self.refresh_authorization();
        if invitation.anchor != self.anchor
            || self
                .store
                .nodes()
                .iter()
                .any(|(id, _)| *id == self.node_key.verifying_key())
        {
            return Err(NodeError::InvalidData);
        }
        if let Some(binding) = self.recovery {
            if !self.has_completed_root_recovery(binding).await? {
                return Err(NodeError::InvalidData);
            }
            // Seal locally saved completion before publishing the ordinary
            // binding, including a prior exit just after secret pinning. A
            // crash between these writes leaves the old flow safely retryable.
            self.seal_completed_root_recovery(binding).await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::RecoveryRejoinPrepared);
        }
        let binding = JoinBinding {
            invitation,
            request_id: rand::random(),
        };
        self.persist_join_binding(binding)?;
        #[cfg(feature = "replica-test-hooks")]
        if self.recovery.is_some() {
            self.store
                .observe(crate::management_store::RecordCheckpoint::RecoveryRejoinPublished);
        }
        self.recovery = None;
        Ok(())
    }

    /// Continue incomplete admission with a trusted new code in this same
    /// canonical network. This only changes the delivery contact; actual TLS
    /// identity, code consumption and current signed membership are checked by
    /// the ordinary precode exchange. Repeating the same contact preserves the
    /// persisted attempt, including across reopen, rather than consuming again.
    pub async fn begin_precode_completion(
        &mut self,
        code: &crate::precode::Precode,
    ) -> Result<(), NodeError> {
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let previous = self.join.ok_or(NodeError::InvalidData)?;
        let invitation = code.invitation();
        if invitation.anchor != self.anchor {
            return Err(NodeError::InvalidData);
        }
        if previous.invitation == invitation {
            return Ok(());
        }
        if self.admission_stage() == crate::manual_admission::AdmissionStage::Complete {
            return Err(NodeError::InvalidData);
        }
        let binding = JoinBinding {
            invitation,
            request_id: previous.request_id,
        };
        self.persist_join_binding(binding)
    }

    fn persist_join_binding(&mut self, binding: JoinBinding) -> Result<(), NodeError> {
        let temporary = self
            .directory
            .join(format!(".join-binding-{:032x}", rand::random::<u128>()));
        let result = (|| {
            write_private(&temporary, &binding.encode())?;
            fs::rename(&temporary, self.directory.join(JOIN_BINDING))?;
            File::open(&self.directory)?.sync_all()?;
            Ok::<_, NodeError>(())
        })();
        let _ = fs::remove_file(&temporary);
        result?;
        self.join = Some(binding);
        Ok(())
    }

    pub(crate) async fn save_admission_material(
        &mut self,
        scope: AdmissionScope,
        secret: AdmissionSecret,
    ) -> Result<(), NodeError> {
        let mut bytes = [0; 40];
        bytes[..8].copy_from_slice(&scope.secret_generation().to_be_bytes());
        bytes[8..].copy_from_slice(secret.saved_bytes());
        let path = self.directory.join(ADMISSION_MATERIAL);
        if path.exists() {
            if read_array::<40>(&path)? != bytes {
                return Err(NodeError::InvalidData);
            }
        } else {
            // Publish a complete fsynced private file atomically. A process
            // exit during the write leaves only its unreferenced temp file.
            let temporary = self.directory.join(format!(
                ".admission-material-{:032x}",
                rand::random::<u128>()
            ));
            let result = (|| {
                write_private(&temporary, &bytes)?;
                fs::hard_link(&temporary, &path)?;
                Ok::<_, NodeError>(())
            })();
            let _ = fs::remove_file(&temporary);
            result?;
        }
        // Also sync on retry: a previous process could have exited after
        // publishing the link but before making its directory entry durable.
        File::open(&self.directory)?.sync_all()?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::SecretFilePublished);
        self.store.set_current_admission(&scope, &secret).await?;
        self.gate.rotate_secret(scope.clone(), secret.clone());
        self.scope = scope;
        self.secret = Some(secret);
        self.refresh_authorization();
        Ok(())
    }

    pub(crate) fn saved_genesis_credential(&self) -> Result<GenesisCredential, NodeError> {
        let credential =
            SavedGenesisCredential(read_array::<64>(&self.directory.join(GENESIS_CREDENTIAL))?)
                .decode();
        if credential.anchor() != self.anchor {
            return Err(NodeError::InvalidCredential);
        }
        Ok(credential)
    }

    pub async fn close(self) {
        self.store.close().await;
    }
}

async fn copy_node_backup(source: &Path, target: &Path, restoring: bool) -> Result<(), NodeError> {
    ensure_absent(target)?;
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".miopunch-backup-{:032x}", rand::random::<u128>()));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(&staging)?;
    let prepare = async {
        for name in [
            NODE_KEY,
            GENESIS_CREDENTIAL,
            ANCHOR,
            ADMISSION_SECRET,
            MANAGEMENT_DB,
            JOIN_BINDING,
            ADMISSION_MATERIAL,
            RECOVERY_BINDING,
        ] {
            let path = source.join(name);
            if path.exists() {
                write_private(&staging.join(name), &fs::read(path)?)?;
            }
        }
        // Replay verifies the copied signed history/anchor/material before
        // publishing anything; restoration's stop is committed in this copy.
        let node = NativeNetwork::open(&staging).await?;
        if restoring {
            sqlx::query("UPDATE precodes SET stopped=1")
                .execute(&node.store.pool)
                .await
                .map_err(StoreError::from)?;
        } else {
            write_private(&staging.join("backup.version"), b"miopunch node backup 1\n")?;
        }
        node.close().await;
        File::open(&staging)?.sync_all()?;
        ensure_absent(target)?;
        publish_new_directory(&staging, target, parent)?;
        File::open(parent)?.sync_all()?;
        Ok::<_, NodeError>(())
    }
    .await;
    if prepare.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    prepare
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
