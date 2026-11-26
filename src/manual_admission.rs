//! Trusted manual invitation and durable approved-result delivery. TLS node
//! authentication, signed management replay and the common admission gate are
//! the security boundaries; invitations contain no shared secret.

use std::time::Duration;

use crate::control_protocol::{AuthenticatedControl, ControlOwner, OwnedControl};
use iroh::endpoint::{ControlExchange, ControlMessageFormat};
use iroh_base::EndpointId;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::{Access, GroupsExtensionArgs};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};
use thiserror::Error;

use crate::admission_gate::{ADMISSION_REQUEST_ALPN, PRECODE_REQUEST_ALPN, ProtocolScope};
use crate::admission_progress::{
    AdmissionReceipt, ApplicationProgress, RECEIPT_DONE, RECEIPT_PROVE, RECEIPT_STOP,
};
use crate::admission_proof::{AdmissionScope, AdmissionSecret};
use crate::management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor};
use crate::management_store::{ManagementRecord, StoreError};
use crate::native_network::{NativeNetwork, NodeError};

const TIMEOUT: Duration = Duration::from_secs(12);

pub(crate) fn admission_deadline(
    connection: &(impl OwnedControl + ?Sized),
) -> n0_future::time::Instant {
    let ceiling = n0_future::time::Instant::now() + TIMEOUT;
    connection
        .caller_deadline()
        .map_or(ceiling, |deadline| deadline.min(ceiling))
}
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionStage {
    AwaitingApproval,
    Approved,
    PendingMaterials,
    Complete,
}

/// Public material personally passed through a trusted channel. Transport
/// hints can be supplied through existing connection configuration separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManualInvitation {
    pub anchor: NetworkAnchor,
    pub contact: EndpointId,
}

impl ManualInvitation {
    pub fn encode(&self) -> [u8; 96] {
        let mut bytes = [0; 96];
        bytes[..32].copy_from_slice(self.anchor.root.as_bytes());
        bytes[32..64].copy_from_slice(self.anchor.genesis.as_bytes());
        bytes[64..].copy_from_slice(self.contact.as_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8; 96]) -> Result<Self, ManualAdmissionError> {
        Ok(Self {
            anchor: NetworkAnchor {
                root: VerifyingKey::from_bytes(&bytes[..32].try_into().unwrap())
                    .map_err(|_| ManualAdmissionError::Rejected)?,
                genesis: Hash::from_bytes(bytes[32..64].try_into().unwrap()),
            },
            contact: EndpointId::from_bytes(&bytes[64..].try_into().unwrap())
                .map_err(|_| ManualAdmissionError::Rejected)?,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct JoinBinding {
    pub invitation: ManualInvitation,
    pub request_id: [u8; 16],
}

impl JoinBinding {
    pub(crate) fn encode(&self) -> [u8; 112] {
        let mut bytes = [0; 112];
        bytes[..96].copy_from_slice(&self.invitation.encode());
        bytes[96..].copy_from_slice(&self.request_id);
        bytes
    }

    pub(crate) fn decode(bytes: &[u8; 112]) -> Result<Self, ManualAdmissionError> {
        Ok(Self {
            invitation: ManualInvitation::decode(&bytes[..96].try_into().unwrap())?,
            request_id: bytes[96..].try_into().unwrap(),
        })
    }
}

/// The pair, rather than a caller-supplied claimed identity alone, identifies
/// an admission attempt. Approval requires a previously authenticated receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionRequest {
    pub applicant: VerifyingKey,
    pub request_id: [u8; 16],
}

/// Shared durable result for approved membership and resumable delivery.
/// `Complete` here records the applicant's durable-save acknowledgement; its
/// own local state remains the authoritative admission-success observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedAdmission {
    pub request: AdmissionRequest,
    pub authorization: Hash,
    pub stage: AdmissionStage,
}

pub enum ManualExchange {
    AwaitingApproval,
    Materials(AdmissionDelivery),
}

/// Authorization has been verified and saved. Consuming this handle receives
/// the second material on this same TLS connection. Dropping it cannot report
/// successful admission; another request can resume the original result.
pub struct AdmissionDelivery {
    connection: ControlOwner,
    deadline: n0_future::time::Instant,
    exchange: ControlExchange,
    local: VerifyingKey,
    binding: JoinBinding,
    request_id: [u8; 16],
    authorization: Hash,
    generation: u64,
}

#[derive(Debug, Error)]
pub enum ManualAdmissionError {
    #[error("this manual request was declined; submit a new request explicitly")]
    RequestDeclined,
    #[error("manual admission rejected")]
    Rejected,
    #[error("current admission path unavailable; durable progress is retained")]
    PathUnavailable,
    #[error("admission attempt timed out; durable progress is retained")]
    TimedOut,
    #[error("manual admission storage failed: {0}")]
    Store(#[from] StoreError),
    #[error("manual admission node persistence failed: {0}")]
    Node(#[from] NodeError),
}

fn tls_key(id: EndpointId) -> Result<VerifyingKey, ManualAdmissionError> {
    VerifyingKey::from_bytes(id.as_bytes()).map_err(|_| ManualAdmissionError::Rejected)
}

// One admission wire implementation for standalone networks and the running SDK.
// Only the running SDK shares its node with other requests and observers.
enum AdmissionNode<'a> {
    Direct(&'a mut NativeNetwork),
    Shared(&'a tokio::sync::Mutex<NativeNetwork>),
}

enum AdmissionNodeGuard<'a> {
    Direct(&'a mut NativeNetwork),
    Shared(tokio::sync::MutexGuard<'a, NativeNetwork>),
}

impl AdmissionNode<'_> {
    async fn lock(&mut self) -> AdmissionNodeGuard<'_> {
        match self {
            Self::Direct(node) => AdmissionNodeGuard::Direct(node),
            Self::Shared(node) => AdmissionNodeGuard::Shared(node.lock().await),
        }
    }
}

impl std::ops::Deref for AdmissionNodeGuard<'_> {
    type Target = NativeNetwork;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Direct(node) => node,
            Self::Shared(node) => node,
        }
    }
}

impl std::ops::DerefMut for AdmissionNodeGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Direct(node) => node,
            Self::Shared(node) => node,
        }
    }
}

pub(crate) async fn serve_shared_admission(
    node: &tokio::sync::Mutex<NativeNetwork>,
    connection: &(impl OwnedControl + ?Sized),
    precode: bool,
) -> Result<AdmissionStage, ManualAdmissionError> {
    let deadline = admission_deadline(connection);
    tokio::time::timeout(
        deadline.saturating_duration_since(n0_future::time::Instant::now()),
        serve_admission_exchange(AdmissionNode::Shared(node), connection, precode),
    )
    .await
    .unwrap_or(Err(ManualAdmissionError::PathUnavailable))
}

pub(crate) async fn serve_admission_receipt(
    node: &tokio::sync::Mutex<NativeNetwork>,
    connection: &ControlOwner,
) -> Result<(), ManualAdmissionError> {
    let deadline = admission_deadline(connection);
    let (exchange, receipt) = admission_io(deadline, async {
        let exchange = connection
            .accept_control_exchange()
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        let mut wire = [0; 80];
        read_fixed(&exchange, &mut wire).await?;
        Ok((exchange, AdmissionReceipt::decode(wire)))
    })
    .await?;
    let (approved, material) = {
        let mut node = node.lock().await;
        node.recover_admission_state().await?;
        if !node
            .gate
            .allows(connection, ProtocolScope::AdmissionRequest)
            || receipt.genesis != node.anchor.genesis
        {
            return Err(ManualAdmissionError::Rejected);
        }
        let request = AdmissionRequest {
            applicant: tls_key(connection.remote_id())?,
            request_id: receipt.request_id,
        };
        let approved = node
            .store
            .approved_admission(&request)
            .await?
            .filter(|approved| approved.authorization == receipt.authorization)
            .ok_or(ManualAdmissionError::Rejected)?;
        let material = match node.valid_approval(&approved) {
            Err(_) => None,
            Ok(()) if approved.stage == AdmissionStage::Approved => {
                return Err(ManualAdmissionError::Rejected);
            }
            Ok(()) => Some((
                node.scope.clone(),
                node.secret.clone().ok_or(ManualAdmissionError::Rejected)?,
            )),
        };
        (approved, material)
    };
    let held_proof;
    let response = if let Some((scope, secret)) = material {
        held_proof = admission_io(deadline, async {
            write_fixed(&exchange, RECEIPT_PROVE).await?;
            crate::admission_proof::prove_server(connection, &scope, &secret)
                .await
                .map_err(|_| ManualAdmissionError::Rejected)
        })
        .await?;
        let node = node.lock().await;
        if !held_proof.applies_to(connection, &node.scope) {
            return Err(ManualAdmissionError::Rejected);
        }
        if node.valid_approval(&approved).is_err() {
            RECEIPT_STOP
        } else {
            node.store
                .advance_admission_delivery(&approved, AdmissionStage::Complete)
                .await?;
            #[cfg(feature = "replica-test-hooks")]
            node.store
                .observe(crate::management_store::RecordCheckpoint::DeliveryComplete);
            RECEIPT_DONE
        }
    } else {
        RECEIPT_STOP
    };
    admission_io(deadline, async {
        write_fixed(&exchange, response).await?;
        exchange
            .finish_send()
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        connection.revocation_handle().closed().await;
        Ok(())
    })
    .await
}

pub(crate) async fn submit_admission_receipt(
    connection: &ControlOwner,
    receipt: AdmissionReceipt,
    scope: &AdmissionScope,
    secret: &AdmissionSecret,
) -> Result<(), ManualAdmissionError> {
    tokio::time::timeout_at(admission_deadline(connection), async {
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        write_fixed(&exchange, &receipt.encode()).await?;
        exchange
            .finish_send()
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        let mut reply = [0; 4];
        read_fixed(&exchange, &mut reply).await?;
        if &reply == RECEIPT_STOP {
            return Ok(());
        }
        if &reply != RECEIPT_PROVE {
            return Err(ManualAdmissionError::Rejected);
        }
        let _proof = crate::admission_proof::prove_client(connection, scope, secret)
            .await
            .map_err(|_| ManualAdmissionError::Rejected)?;
        read_fixed(&exchange, &mut reply).await?;
        match &reply {
            RECEIPT_DONE | RECEIPT_STOP => Ok(()),
            _ => Err(ManualAdmissionError::Rejected),
        }
    })
    .await
    .unwrap_or(Err(ManualAdmissionError::TimedOut))
}

async fn serve_admission_exchange(
    mut owner: AdmissionNode<'_>,
    connection: &(impl OwnedControl + ?Sized),
    precode: bool,
) -> Result<AdmissionStage, ManualAdmissionError> {
    owner.lock().await.restricted_connection(connection)?;
    let exchange = connection
        .accept_control_exchange()
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    let wire = crate::admission_request_wire::read(&exchange, connection.remote_id(), precode)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::InvalidData => {
                ManualAdmissionError::Rejected
            }
            _ => ManualAdmissionError::PathUnavailable,
        })?;
    let binding = JoinBinding::decode(&wire.binding)?;
    let request = AdmissionRequest {
        applicant: wire.applicant,
        request_id: binding.request_id,
    };
    let (approved, history, generation) = {
        let mut node = owner.lock().await;
        node.restricted_connection(connection)?;
        node.recover_admission_state().await?;
        node.store.ensure_admissions_open().await?;
        node.restricted_connection(connection)?;
        if binding.invitation != node.manual_invitation()? {
            return Err(ManualAdmissionError::Rejected);
        }
        let approved = if let Some(token) = wire.token {
            Some(node.redeem_precode(token, &request).await?)
        } else {
            node.store.record_admission_request(&request).await?;
            node.store.approved_admission(&request).await?
        };
        if let Some(name) = wire.name {
            node.update_device_names(|names| {
                names.remember(name);
                Ok(())
            })
            .map_err(NodeError::from)?;
        }
        let Some(approved) = approved else {
            let rejected = node.store.admission_rejected(&request).await?;
            drop(node);
            write_fixed(&exchange, &[if rejected { 2 } else { 0 }])
                .await
                .map_err(|_| ManualAdmissionError::PathUnavailable)?;
            exchange
                .finish_send()
                .await
                .map_err(|_| ManualAdmissionError::PathUnavailable)?;
            return Ok(AdmissionStage::AwaitingApproval);
        };
        node.store.apply_ready().await?;
        node.refresh_authorization();
        node.valid_approval(&approved)?;
        let records = node.store.applied_records().await?;
        if records.is_empty() || records.len() > MAX_RECORDS {
            return Err(ManualAdmissionError::Rejected);
        }
        let raw: Vec<_> = records
            .iter()
            .map(|record| (record.header(), record.body()))
            .collect();
        node.store.ensure_admissions_open().await?;
        let history = p2panda_core::cbor::encode_cbor(&(approved.authorization, raw))
            .map_err(|_| ManualAdmissionError::Rejected)?;
        if history.len() > MAX_HISTORY_BYTES {
            return Err(ManualAdmissionError::Rejected);
        }
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationPrepared);
        (approved, history, node.scope.secret_generation())
    };
    write_fixed(&exchange, &[1])
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    write_fixed(&exchange, &generation.to_be_bytes())
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    write_frame(&exchange, &history).await?;
    #[cfg(feature = "replica-test-hooks")]
    owner
        .lock()
        .await
        .store
        .observe(crate::management_store::RecordCheckpoint::AuthorizationSent);
    let mut ack = [0; 4];
    read_fixed(&exchange, &mut ack)
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    if &ack != b"AUTH" {
        return Err(ManualAdmissionError::Rejected);
    }

    {
        let node = owner.lock().await;
        node.store
            .advance_admission_delivery(&approved, AdmissionStage::PendingMaterials)
            .await?;
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationAcknowledged);
        // Keep authorization and credential through their short release, but
        // relinquish the node while awaiting the applicant's durable receipt.
        node.restricted_connection(connection)?;
        node.store.ensure_admissions_open().await?;
        node.valid_approval(&approved)?;
        let secret = node.secret.as_ref().ok_or(ManualAdmissionError::Rejected)?;
        if node.scope.secret_generation() != generation
            || !node
                .store
                .current_admission_matches(&node.scope, &secret.fingerprint(&node.scope))
                .await?
        {
            return Err(ManualAdmissionError::Rejected);
        }
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::SecretPrepared);
        write_fixed(&exchange, &generation.to_be_bytes())
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        write_fixed(&exchange, secret.saved_bytes())
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::SecretSent);
    }
    read_fixed(&exchange, &mut ack)
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    if &ack != b"SAVE" {
        return Err(ManualAdmissionError::Rejected);
    }
    {
        let node = owner.lock().await;
        node.valid_approval(&approved)?;
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::CompletionAcknowledged);
        node.store
            .advance_admission_delivery(&approved, AdmissionStage::Complete)
            .await?;
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::DeliveryComplete);
    }
    write_fixed(&exchange, b"DONE")
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    exchange
        .finish_send()
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    Ok(AdmissionStage::Complete)
}

impl NativeNetwork {
    pub(crate) async fn recover_admission_state(&mut self) -> Result<(), ManualAdmissionError> {
        self.store.apply_ready().await?;
        self.refresh_authorization();
        Ok(())
    }

    pub(crate) fn is_admin(&self, key: VerifyingKey) -> bool {
        self.store.nodes().contains(&(key, Access::manage()))
    }

    pub(crate) fn is_member(&self, key: VerifyingKey) -> bool {
        self.store.nodes().iter().any(|(node, _)| *node == key)
    }

    pub(crate) fn restricted_connection(
        &self,
        connection: &(impl OwnedControl + ?Sized),
    ) -> Result<(), ManualAdmissionError> {
        if !matches!(
            connection.alpn(),
            ADMISSION_REQUEST_ALPN | PRECODE_REQUEST_ALPN
        ) || !self
            .gate
            .allows(connection, ProtocolScope::AdmissionRequest)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        Ok(())
    }

    pub fn manual_invitation(&self) -> Result<ManualInvitation, ManualAdmissionError> {
        if !self.is_admin(self.node_key.verifying_key()) {
            return Err(ManualAdmissionError::Rejected);
        }
        Ok(ManualInvitation {
            anchor: self.anchor,
            contact: iroh_base::SecretKey::from_bytes(self.node_key.as_bytes()).public(),
        })
    }

    /// Local success never depends on a sender's progress flag or global
    /// convergence. It requires current verified membership and the credential
    /// which this node committed durably before installing it in its gate.
    pub fn admission_stage(&self) -> AdmissionStage {
        if self.is_member(self.node_key.verifying_key()) {
            if self.secret.is_some() {
                AdmissionStage::Complete
            } else {
                AdmissionStage::PendingMaterials
            }
        } else {
            AdmissionStage::AwaitingApproval
        }
    }

    pub async fn manual_requests(&self) -> Result<Vec<AdmissionRequest>, ManualAdmissionError> {
        Ok(self.store.admission_requests().await?)
    }

    /// A rejection ends this authenticated attempt, not the device identity.
    /// Already granted membership cannot be undone through this operation.
    pub async fn reject_manual_admission(
        &mut self,
        request: &AdmissionRequest,
    ) -> Result<(), ManualAdmissionError> {
        self.recover_admission_state().await?;
        self.store.ensure_admissions_open().await?;
        if !self.is_admin(self.node_key.verifying_key())
            || !self.store.admission_requests().await?.contains(request)
            || self.store.approved_admission(request).await?.is_some()
        {
            return Err(ManualAdmissionError::Rejected);
        }
        self.store.save_admission_rejection(request).await?;
        Ok(())
    }

    pub async fn manual_request_declined(&self) -> Result<bool, ManualAdmissionError> {
        let Some(binding) = self.join else {
            return Ok(false);
        };
        Ok(self
            .store
            .admission_rejected(&AdmissionRequest {
                applicant: self.node_key.verifying_key(),
                request_id: binding.request_id,
            })
            .await?)
    }

    /// Shared read boundary for an approved result and its delivery progress.
    /// Invalidated grants are rejected even when a new grant later restores
    /// the same stable identity's membership.
    pub async fn approved_admission(
        &mut self,
        request: &AdmissionRequest,
    ) -> Result<Option<ApprovedAdmission>, ManualAdmissionError> {
        self.recover_admission_state().await?;
        let approved = self.store.approved_admission(request).await?;
        if let Some(approved) = &approved {
            self.valid_approval(approved)?;
        }
        Ok(approved)
    }

    /// Approve only an authenticated request, and only ordinary membership.
    /// A retry first applies the durable original result and checks its current
    /// authorization rather than blindly adding the same identity again.
    pub async fn approve_manual_admission(
        &mut self,
        request: &AdmissionRequest,
    ) -> Result<ApprovedAdmission, ManualAdmissionError> {
        // A cancelled approval for another request of this identity may have
        // already committed its grant. Recover it before role/member/dedup
        // decisions, not only after looking up this exact request ID.
        self.recover_admission_state().await?;
        self.store.ensure_admissions_open().await?;
        if !self.is_admin(self.node_key.verifying_key())
            || !self.store.admission_requests().await?.contains(request)
            || self.store.admission_rejected(request).await?
            || request.applicant == self.anchor.root
        {
            return Err(ManualAdmissionError::Rejected);
        }
        if let Some(approved) = self.store.approved_admission(request).await? {
            self.valid_approval(&approved)?;
            return Ok(approved);
        }
        let record = self.admission_record(request).await?;
        self.store.save_admission_approval(request, &record).await?;
        self.store.apply_ready().await?;
        self.refresh_authorization();
        let approved = self
            .store
            .approved_admission(request)
            .await?
            .ok_or(ManualAdmissionError::Rejected)?;
        self.valid_approval(&approved)?;
        Ok(approved)
    }

    /// Shared authoring seam for manual approval and precode consumption.
    /// The caller commits this signed result with its own durable request.
    pub(crate) async fn admission_record(
        &self,
        request: &AdmissionRequest,
    ) -> Result<ManagementRecord, ManualAdmissionError> {
        let record = if self.is_member(request.applicant) {
            // A new delivery request for a valid member reuses signed material,
            // rather than issuing a duplicate membership Add.
            self.store
                .applied_records()
                .await?
                .into_iter()
                .rev()
                .find(|record| {
                    self.store.membership_authorization_valid(
                        Hash::digest(record.header()),
                        request.applicant,
                    )
                })
                .ok_or(ManualAdmissionError::Rejected)?
        } else {
            let log_id = loop {
                let candidate = rand::random();
                if !self
                    .store
                    .has_author_log_id(self.node_key.verifying_key(), candidate)
                {
                    break candidate;
                }
            };
            ManagementRecord::new(
                Header::<ManagementExtension>::builder()
                    .build(
                        &SigningKey::from_bytes(self.node_key.as_bytes()),
                        ManagementExtension {
                            version: 1,
                            log_id,
                            group: GroupsExtensionArgs {
                                group_id: self.anchor.root,
                                action: GroupAction::Add {
                                    member: GroupMember::Individual(request.applicant),
                                    access: Access::read(),
                                },
                                dependencies: self.store.tips(),
                            },
                        },
                    )
                    .encode(),
                None,
            )
        };
        if !self.is_member(request.applicant) {
            self.store.validate_local_record(&record)?;
        }
        Ok(record)
    }

    pub(crate) fn valid_approval(
        &self,
        approved: &ApprovedAdmission,
    ) -> Result<(), ManualAdmissionError> {
        if !self.is_admin(self.node_key.verifying_key())
            || !self
                .store
                .membership_authorization_valid(approved.authorization, approved.request.applicant)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        Ok(())
    }

    /// Handle one authenticated request or delivery retry. Awaiting approval
    /// returns promptly; the administrator approves through the public command
    /// above, and the applicant repeats this same durable identity/request.
    pub async fn serve_manual_admission(
        &mut self,
        connection: &(impl OwnedControl + ?Sized),
    ) -> Result<AdmissionStage, ManualAdmissionError> {
        let deadline = admission_deadline(connection);
        let result = tokio::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.serve_admission_inner(connection, false),
        )
        .await
        .unwrap_or(Err(ManualAdmissionError::PathUnavailable));
        if result.is_err() {
            connection.close(0u32.into(), b"manual admission ended");
        }
        result
    }

    pub(crate) async fn serve_admission_inner(
        &mut self,
        connection: &(impl OwnedControl + ?Sized),
        precode: bool,
    ) -> Result<AdmissionStage, ManualAdmissionError> {
        serve_admission_exchange(AdmissionNode::Direct(self), connection, precode).await
    }

    /// Receive and save verified authorization, leaving secret delivery as a
    /// distinct stage. The same invitation holder using a different TLS key is
    /// a different applicant and cannot retrieve this result.
    pub async fn request_manual_admission(
        &mut self,
        connection: &(impl OwnedControl + ?Sized),
    ) -> Result<ManualExchange, ManualAdmissionError> {
        let deadline = admission_deadline(connection);
        let result = tokio::time::timeout(
            deadline.saturating_duration_since(n0_future::time::Instant::now()),
            self.request_admission_inner(connection, None, deadline),
        )
        .await
        .unwrap_or(Err(ManualAdmissionError::TimedOut));
        if result.is_err() {
            connection.close(0u32.into(), b"manual admission ended");
        }
        result
    }

    pub(crate) async fn request_admission_inner(
        &mut self,
        connection: &(impl OwnedControl + ?Sized),
        token: Option<[u8; 32]>,
        deadline: n0_future::time::Instant,
    ) -> Result<ManualExchange, ManualAdmissionError> {
        self.recover_admission_state().await?;
        self.store.ensure_admissions_open().await?;
        self.restricted_connection(connection)?;
        let binding = self.join.ok_or(ManualAdmissionError::Rejected)?;
        if token.is_none() && self.manual_request_declined().await? {
            return Err(ManualAdmissionError::RequestDeclined);
        }
        if binding.invitation.anchor != self.anchor
            || connection.remote_id() != binding.invitation.contact
        {
            return Err(ManualAdmissionError::Rejected);
        }
        let exchange = connection
            .open_control_exchange()
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        // A fresh code is a distinct redemption even when this joining node
        // retains its stable persisted admission attempt. Manual requests keep
        // their original namespace. The opaque delivery handle retains the
        // original local binding for the second-material verification.
        let mut wire_binding = binding;
        if let Some(token) = token {
            wire_binding.request_id =
                crate::admission_request_id::precode_request_id(binding.request_id, token);
        }
        let local = self.node_key.verifying_key();
        crate::admission_request_wire::write(
            &exchange,
            &wire_binding.encode(),
            local,
            token,
            self.device_names.record(local),
        )
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        let mut status = [0];
        read_fixed(&exchange, &mut status)
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        if matches!(status, [0] | [1]) {
            // The authenticated provider sends these replies after it commits
            // the request or redemption. This survives loss of later frames.
            self.store.save_local_admission_progress(wire_binding.request_id, ApplicationProgress::Delivered).await?;
        }
        if status == [0] {
            exchange
                .finish_send()
                .await
                .map_err(|_| ManualAdmissionError::PathUnavailable)?;
            return Ok(ManualExchange::AwaitingApproval);
        }
        if status == [2] && token.is_none() {
            self.store
                .save_admission_rejection(&AdmissionRequest {
                    applicant: self.node_key.verifying_key(),
                    request_id: binding.request_id,
                })
                .await?;
            return Err(ManualAdmissionError::RequestDeclined);
        }
        if status != [1] {
            return Err(ManualAdmissionError::Rejected);
        }
        let mut generation = [0; 8];
        read_fixed(&exchange, &mut generation)
            .await
            .map_err(|_| ManualAdmissionError::PathUnavailable)?;
        let generation = u64::from_be_bytes(generation);
        if generation > i64::MAX as u64 || generation < self.scope.secret_generation() {
            return Err(ManualAdmissionError::Rejected);
        }
        let history = read_frame(&exchange).await?;
        let (authorization, raw): (Hash, Vec<(Vec<u8>, Option<Vec<u8>>)>) =
            p2panda_core::cbor::decode_cbor(history.as_slice())
                .map_err(|_| ManualAdmissionError::Rejected)?;
        if raw.is_empty()
            || raw.len() > MAX_RECORDS
            || Hash::digest(&raw[0].0) != self.anchor.genesis
            || !raw
                .iter()
                .any(|(header, _)| Hash::digest(header) == authorization)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        let auth = ManagementAuth::replay_records(self.anchor, &raw)
            .map_err(|_| ManualAdmissionError::Rejected)?;
        let local = self.node_key.verifying_key();
        if !auth
            .nodes()
            .contains(&(tls_key(connection.remote_id())?, Access::manage()))
            || !auth.membership_authorization_valid(authorization, local)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        // The authenticated provider's generation becomes durable before the
        // new grant can expose normal membership. An old saved secret does not
        // complete a fresh admission when this provider uses a newer one.
        self.store
            .save_expected_admission_generation(binding.request_id, generation)
            .await?;
        // Bind the exact signed grant before applying it. On reopen, the grant
        // counts only if the local store actually committed its authority.
        self.store
            .save_local_admission_authorization(wire_binding.request_id, authorization)
            .await?;
        if generation > self.scope.secret_generation() {
            self.gate.suspend_secret();
            self.secret = None;
        }
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationVerified);
        for (header, body) in raw {
            self.store
                .ingest(&ManagementRecord::new(header, body.as_deref()))
                .await?;
        }
        self.store.apply_ready().await?;
        self.refresh_authorization();
        if !self
            .store
            .membership_authorization_valid(authorization, local)
            || !self.is_admin(tls_key(connection.remote_id())?)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        self.store.save_local_admission_progress(wire_binding.request_id, ApplicationProgress::Authorized).await?;
        #[cfg(feature = "replica-test-hooks")]
        self.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationSaved);
        Ok(ManualExchange::Materials(AdmissionDelivery {
            connection: connection.retain_control(),
            deadline,
            exchange,
            local,
            binding,
            request_id: wire_binding.request_id,
            authorization,
            generation,
        }))
    }

    /// Save the current admission secret before installing it in the gate or
    /// acknowledging completion. A path failure after saving both materials
    /// leaves local success intact, while the sender can retry its pending ack.
    pub async fn complete_manual_admission(
        &mut self,
        delivery: AdmissionDelivery,
    ) -> Result<AdmissionStage, ManualAdmissionError> {
        let result = async {
            self.recover_admission_state().await?;
            self.store.ensure_admissions_open().await?;
            self.restricted_connection(&delivery.connection)?;
            if self.node_key.verifying_key() != delivery.local
                || self.join.map(|binding| binding.encode()) != Some(delivery.binding.encode())
                || self.anchor != delivery.binding.invitation.anchor
                || delivery.connection.remote_id() != delivery.binding.invitation.contact
                || !self.is_member(delivery.local)
                || !self.is_admin(tls_key(delivery.connection.remote_id())?)
                || !self
                    .store
                    .membership_authorization_valid(delivery.authorization, delivery.local)
            {
                return Err(ManualAdmissionError::Rejected);
            }
            let bytes = admission_io(delivery.deadline, async {
                write_fixed(&delivery.exchange, b"AUTH").await?;
                let mut bytes = [0; 40];
                read_fixed(&delivery.exchange, &mut bytes).await?;
                Ok(bytes)
            })
            .await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::SecretReceived);
            let scope = AdmissionScope::new(
                *self.anchor.root.as_bytes(),
                u64::from_be_bytes(bytes[..8].try_into().unwrap()),
            );
            if scope.secret_generation() != delivery.generation {
                return Err(ManualAdmissionError::Rejected);
            }
            let secret = AdmissionSecret::from_saved(bytes[8..].try_into().unwrap());
            let receipt = AdmissionReceipt {
                genesis: self.anchor.genesis,
                request_id: delivery.request_id,
                authorization: delivery.authorization,
            };
            let contact = crate::client_contact::load_invitation(self)
                .map_err(NodeError::from)?
                .filter(|invitation| invitation.contact() == delivery.binding.invitation.contact)
                .map(|invitation| invitation.address)
                .unwrap_or_else(|| iroh::EndpointAddr::new(delivery.binding.invitation.contact));
            self.commit_admission_material(scope, secret, Some((receipt, &contact)))
                .await?;
            #[cfg(feature = "replica-test-hooks")]
            self.store
                .observe(crate::management_store::RecordCheckpoint::SecretSaved);

            admission_io(delivery.deadline, async {
                write_fixed(&delivery.exchange, b"SAVE")
                    .await
                    .map_err(|_| ManualAdmissionError::PathUnavailable)?;
                #[cfg(feature = "replica-test-hooks")]
                self.store
                    .observe(crate::management_store::RecordCheckpoint::CompletionSent);
                delivery
                    .exchange
                    .finish_send()
                    .await
                    .map_err(|_| ManualAdmissionError::PathUnavailable)?;
                let mut ack = [0; 4];
                read_fixed(&delivery.exchange, &mut ack)
                    .await
                    .map_err(|_| ManualAdmissionError::PathUnavailable)?;
                if &ack != b"DONE" {
                    return Err(ManualAdmissionError::Rejected);
                }
                Ok(())
            })
            .await?;
            self.store.settle_admission_receipt(receipt).await?;
            Ok(self.admission_stage())
        }
        .await;
        if result.is_err() {
            delivery
                .connection
                .close(0u32.into(), b"manual admission ended");
        }
        result
    }
}

async fn admission_io<T>(
    deadline: n0_future::time::Instant,
    work: impl Future<Output = Result<T, ManualAdmissionError>>,
) -> Result<T, ManualAdmissionError> {
    if deadline <= n0_future::time::Instant::now() {
        return Err(ManualAdmissionError::TimedOut);
    }
    tokio::time::timeout_at(deadline, work)
        .await
        .unwrap_or(Err(ManualAdmissionError::TimedOut))
}

async fn write_fixed(exchange: &ControlExchange, bytes: &[u8]) -> Result<(), ManualAdmissionError> {
    exchange
        .send_message(
            ControlMessageFormat::Fixed {
                length: bytes.len(),
            },
            bytes,
        )
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)
}
async fn read_fixed<const N: usize>(
    exchange: &ControlExchange,
    bytes: &mut [u8; N],
) -> Result<(), ManualAdmissionError> {
    let message = exchange
        .recv_message(ControlMessageFormat::Fixed { length: N })
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    bytes.copy_from_slice(&message);
    Ok(())
}
async fn write_frame(exchange: &ControlExchange, bytes: &[u8]) -> Result<(), ManualAdmissionError> {
    exchange
        .send_message(
            ControlMessageFormat::LengthPrefixed32 {
                maximum: MAX_HISTORY_BYTES,
            },
            bytes,
        )
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)
}
async fn read_frame(exchange: &ControlExchange) -> Result<Vec<u8>, ManualAdmissionError> {
    let message = exchange
        .recv_message(ControlMessageFormat::LengthPrefixed32 {
            maximum: MAX_HISTORY_BYTES,
        })
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                ManualAdmissionError::Rejected
            } else {
                ManualAdmissionError::PathUnavailable
            }
        })?;
    if message.is_empty() {
        return Err(ManualAdmissionError::Rejected);
    }
    Ok(message)
}
