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
    let (approved, raw, generation) = {
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
        if records.is_empty() {
            return Err(ManualAdmissionError::Rejected);
        }
        let raw: Vec<_> = records
            .into_iter()
            .map(|record| (record.header, record.body))
            .collect();
        node.store.ensure_admissions_open().await?;
        #[cfg(feature = "replica-test-hooks")]
        node.store
            .observe(crate::management_store::RecordCheckpoint::AuthorizationPrepared);
        (approved, raw, node.scope.secret_generation())
    };
    let history = crate::admission_history_wire::History::new(approved.authorization, &raw)
        .map_err(|_| ManualAdmissionError::Rejected)?;
    write_fixed(&exchange, &[history.status()])
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    write_fixed(&exchange, &generation.to_be_bytes())
        .await
        .map_err(|_| ManualAdmissionError::PathUnavailable)?;
    history.send(&exchange).await.map_err(history_error)?;
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
        let result = self.request_admission_inner(connection, None, deadline).await;
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
        let (exchange, status) = admission_io(deadline, async {
            let exchange = connection
                .open_control_exchange()
                .await
                .map_err(|_| ManualAdmissionError::PathUnavailable)?;
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
            Ok((exchange, status))
        }).await?;
        if matches!(status, [0] | [1] | [3]) {
            // The authenticated provider sends these replies after it commits
            // the request or redemption. This survives loss of later frames.
            self.store.save_local_admission_progress(wire_binding.request_id, ApplicationProgress::Delivered).await?;
        }
        if status == [0] {
            admission_io(deadline, async {
                exchange.finish_send().await.map_err(|_| ManualAdmissionError::PathUnavailable)
            }).await?;
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
        if !matches!(status, [1] | [3]) {
            return Err(ManualAdmissionError::Rejected);
        }
        let (generation, authorization, raw, verified) = admission_io(deadline, async {
            let mut generation = [0; 8];
            read_fixed(&exchange, &mut generation)
                .await
                .map_err(|_| ManualAdmissionError::PathUnavailable)?;
            let generation = u64::from_be_bytes(generation);
            if generation > i64::MAX as u64 || generation < self.scope.secret_generation() {
                return Err(ManualAdmissionError::Rejected);
            }
            let history_binding = crate::admission_history_wire::HistoryBinding {
                anchor: self.anchor,
                provider: *connection.remote_id().as_bytes(),
                request: wire_binding.request_id,
                generation,
            };
            let previous = if status == [3] { self.store.admission_history_checkpoint().await? } else { None };
            let store = &self.store;
            let (authorization, raw, verified) = crate::admission_history_wire::receive(
                &exchange, status[0], history_binding, previous,
                move |checkpoint| async move { store.save_admission_history(&checkpoint).await.map_err(std::io::Error::other) },
            )
                .await.map_err(history_error)?;
            Ok((generation, authorization, raw, verified))
        }).await?;
        if raw.is_empty()
            || Hash::digest(&raw[0].0) != self.anchor.genesis
            || !raw
                .iter()
                .any(|(header, _)| Hash::digest(header) == authorization)
        {
            return Err(ManualAdmissionError::Rejected);
        }
        let auth = match verified {
            Some(auth) => auth,
            None => ManagementAuth::replay_records(self.anchor, &raw)
                .map_err(|_| ManualAdmissionError::Rejected)?,
        };
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
        if status == [3] {
            self.store.apply_verified_history(&raw, auth).await?;
        } else {
            self.store.apply_batch(&raw).await?;
        }
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
            let saved = crate::management_store::LocalAdmissionDelivery {
                receipt,
                contact: &contact,
                base_request_id: delivery.binding.request_id,
            };
            self.commit_admission_material(scope, secret, Some(saved))
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
fn history_error(error: std::io::Error) -> ManualAdmissionError {
    if error.kind() == std::io::ErrorKind::InvalidData {
        ManualAdmissionError::Rejected
    } else {
        ManualAdmissionError::PathUnavailable
    }
}

#[cfg(test)]
mod paged_history_tests {
    use super::*;
    use std::{net::Ipv4Addr, sync::Arc, time::{SystemTime, UNIX_EPOCH}};

    #[tokio::test]
    async fn manual_and_precode_deliver_full_paged_signed_history_before_material_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let mut admin = NativeNetwork::create(directory.path().join("admin")).await.unwrap();
        let body = vec![7; crate::management_record::MAX_PAGE_BYTES / 3 + 512];
        let mut records = Vec::new();
        for index in 0..3u8 {
            let member = SigningKey::generate().verifying_key();
            let header = Header::<ManagementExtension>::builder().body(&body).build(
                &admin.node_key,
                ManagementExtension {
                    version: 1,
                    log_id: [index; 16],
                    group: GroupsExtensionArgs {
                        group_id: admin.anchor.root,
                        action: GroupAction::Add { member: GroupMember::Individual(member), access: Access::read() },
                        dependencies: vec![admin.anchor.genesis],
                    },
                },
            ).encode();
            records.push((header, Some(body.clone())));
        }
        admin.store.apply_batch(&records).await.unwrap();
        admin.refresh_authorization();
        let expires = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600;
        let code = admin.issue_precode(expires, Some(1)).await.unwrap();
        let provider = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        provider.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
        for precode in [false, true] {
            let path = directory.path().join(if precode { "precode" } else { "manual" });
            let mut joining = NativeNetwork::join(&path, admin.manual_invitation().unwrap()).await.unwrap();
            let endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
            let target = iroh::EndpointAddr::new(provider.id()).with_addrs(provider.addr().addrs);
            let (client, server) = tokio::join!(
                endpoint.connect_control(target, ADMISSION_REQUEST_ALPN),
                async { provider.accept().await.unwrap().accept_control().unwrap().await }
            );
            let (client, server) = (Arc::new(client.unwrap()), Arc::new(server.unwrap()));
            if !precode {
                let (result, stage) = tokio::join!(joining.request_manual_admission(&client), admin.serve_manual_admission(&server));
                assert!(matches!(result.unwrap(), ManualExchange::AwaitingApproval));
                assert_eq!(stage.unwrap(), AdmissionStage::AwaitingApproval);
                let request = admin.manual_requests().await.unwrap().pop().unwrap();
                admin.approve_manual_admission(&request).await.unwrap();
            }
            let receive = async {
                let exchange = if precode {
                    joining.request_precode_admission(&client, &code).await.unwrap()
                } else {
                    joining.request_manual_admission(&client).await.unwrap()
                };
                let ManualExchange::Materials(materials) = exchange else { panic!("approved paged history must deliver materials") };
                assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
                joining.complete_manual_admission(materials).await.unwrap()
            };
            let send = async {
                if precode { admin.serve_precode_admission(&server).await }
                else { admin.serve_manual_admission(&server).await }
            };
            let (received, sent) = tokio::join!(receive, send);
            assert_eq!(received, AdmissionStage::Complete);
            assert_eq!(sent.unwrap(), AdmissionStage::Complete);
            assert_eq!(joining.store.applied_records().await.unwrap(), admin.store.applied_records().await.unwrap());
            let expected = joining.status();
            endpoint.close().await;
            joining.close().await;
            let reopened = NativeNetwork::open(&path).await.unwrap();
            assert_eq!(reopened.admission_stage(), AdmissionStage::Complete);
            assert_eq!(reopened.status(), expected);
            assert_eq!(reopened.store.applied_records().await.unwrap(), admin.store.applied_records().await.unwrap());
            reopened.close().await;
        }
        assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
        assert_eq!(admin.precode_redemptions(&code).await.unwrap()[0].stage, AdmissionStage::Complete);
        provider.close().await;
        admin.close().await;
    }

    #[tokio::test]
    async fn incomplete_paged_admission_never_acknowledges_or_saves_partial_materials() {
        let directory = tempfile::tempdir().unwrap();
        let mut admin = NativeNetwork::create(directory.path().join("admin")).await.unwrap();
        let mut joining = NativeNetwork::join(directory.path().join("joining"), admin.manual_invitation().unwrap()).await.unwrap();
        let provider = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        provider.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
        let target = iroh::EndpointAddr::new(provider.id()).with_addrs(provider.addr().addrs);
        let (client, server) = tokio::join!(
            endpoint.connect_control(target, ADMISSION_REQUEST_ALPN),
            async { provider.accept().await.unwrap().accept_control().unwrap().await }
        );
        let (client, server) = (Arc::new(client.unwrap()), Arc::new(server.unwrap()));
        let (requested, served) = tokio::join!(joining.request_manual_admission(&client), admin.serve_manual_admission(&server));
        assert!(matches!(requested.unwrap(), ManualExchange::AwaitingApproval));
        assert_eq!(served.unwrap(), AdmissionStage::AwaitingApproval);
        let request = admin.manual_requests().await.unwrap().pop().unwrap();
        let approved = admin.approve_manual_admission(&request).await.unwrap();
        let raw = admin.store.applied_records().await.unwrap().into_iter()
            .map(|record| (record.header, record.body)).collect::<Vec<_>>();
        let offer = p2panda_core::cbor::encode_cbor(&(approved.authorization, raw.len() as u64)).unwrap();
        let mut first_page = 0u64.to_be_bytes().to_vec();
        first_page.extend(crate::management_record::encode_raw_records(&raw[..1]).unwrap());
        for cut in 0..5 {
            let deadline = n0_future::time::Instant::now() + Duration::from_millis(350);
            let send = async {
                let exchange = server.accept_control_exchange().await.unwrap();
                crate::admission_request_wire::read(&exchange, server.remote_id(), false).await.unwrap();
                write_fixed(&exchange, &[crate::admission_history_wire::PAGED_HISTORY]).await.unwrap();
                write_fixed(&exchange, &admin.scope.secret_generation().to_be_bytes()).await.unwrap();
                let frame = ControlMessageFormat::LengthPrefixed32 { maximum: crate::management_record::MAX_PAGE_BYTES };
                exchange.send_message(ControlMessageFormat::LengthPrefixed16 { maximum: 256 }, &offer).await.unwrap();
                exchange.recv_message(ControlMessageFormat::Fixed { length: 40 }).await.unwrap();
                write_fixed(&exchange, &0u64.to_be_bytes()).await.unwrap();
                exchange.send_message(frame, &first_page).await.unwrap();
                let durable = exchange.recv_message(ControlMessageFormat::Fixed { length: 40 }).await.unwrap();
                assert_eq!(&durable[..8], &1u64.to_be_bytes());
                match cut {
                    1 => {
                        let mut truncated = 1u64.to_be_bytes().to_vec();
                        truncated.extend([0, 0]);
                        exchange.send_message(frame, &truncated).await.unwrap();
                    },
                    2 => exchange.send_message(frame, &first_page).await.unwrap(),
                    3 => {
                        let mut changed = raw[1..].to_vec();
                        *changed[0].0.last_mut().unwrap() ^= 1;
                        let mut page = 1u64.to_be_bytes().to_vec();
                        page.extend(crate::management_record::encode_raw_records(&changed).unwrap());
                        exchange.send_message(frame, &page).await.unwrap();
                    },
                    _ => {},
                }
                if cut != 4 { let _ = exchange.finish_send().await; }
                let ack = tokio::time::timeout(Duration::from_secs(1), exchange.recv_message(ControlMessageFormat::Fixed { length: 4 })).await;
                assert!(!matches!(ack, Ok(Ok(ref bytes)) if bytes == b"AUTH"), "incomplete history must not release the admission secret");
            };
            let receive = joining.request_admission_inner(&client, None, deadline);
            let (result, ()) = tokio::join!(receive, send);
            if cut == 4 { assert!(matches!(result, Err(ManualAdmissionError::TimedOut))); }
            else { assert!(result.is_err()); }
            assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
            assert_eq!(joining.status().applied_count, 0);
            assert!(joining.secret.is_none());
            let checkpoint = joining.store.admission_history_checkpoint().await.unwrap().unwrap();
            assert_eq!(checkpoint.records, raw[..1]);
            assert!(!checkpoint.complete);
        }
        endpoint.close().await;
        provider.close().await;
        joining.close().await;
        admin.close().await;
    }

    #[tokio::test]
    async fn precode_delivery_crosses_the_record_page_limit_without_truncating_history() {
        let directory = tempfile::tempdir().unwrap();
        let mut admin = NativeNetwork::create(directory.path().join("admin")).await.unwrap();
        let root = admin.saved_genesis_credential().unwrap();
        // Signed no-op promotions keep a small member set while exercising an
        // actual ordered history longer than one record-count page.
        let mut previous = admin.anchor.genesis;
        let records = (0..crate::management_record::MAX_PAGE_RECORDS).map(|index| {
            let header = Header::<ManagementExtension>::builder().build(
                &root.root,
                ManagementExtension {
                    version: 1,
                    log_id: (index as u128).to_be_bytes(),
                    group: GroupsExtensionArgs {
                        group_id: admin.anchor.root,
                        action: GroupAction::Promote {
                            member: GroupMember::Individual(admin.node_key.verifying_key()),
                            access: Access::manage(),
                        },
                        dependencies: vec![previous],
                    },
                },
            ).encode();
            previous = Hash::digest(&header);
            (header, None)
        }).collect::<Vec<_>>();
        admin.store.apply_batch(&records).await.unwrap();
        admin.refresh_authorization();
        let expires = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600;
        let code = admin.issue_precode(expires, Some(1)).await.unwrap();
        let path = directory.path().join("joining");
        let mut joining = NativeNetwork::join(&path, code.invitation()).await.unwrap();
        let provider = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        provider.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()]);
        let mut previous = (0usize, 0u8);
        let mut completed = false;
        for contact in 0..16 {
            let endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
            let target = iroh::EndpointAddr::new(provider.id()).with_addrs(provider.addr().addrs);
            let (client, server) = tokio::join!(
                endpoint.connect_control(target, ADMISSION_REQUEST_ALPN),
                async { provider.accept().await.unwrap().accept_control().unwrap().await }
            );
            let (client, server) = (Arc::new(client.unwrap()), Arc::new(server.unwrap()));
            let receive = async {
                match joining.request_precode_admission(&client, &code).await? {
                    ManualExchange::Materials(materials) => joining.complete_manual_admission(materials).await,
                    _ => Err(ManualAdmissionError::Rejected),
                }
            };
            let (received, sent) = tokio::join!(receive, admin.serve_precode_admission(&server));
            endpoint.close().await;
            if matches!(received, Ok(AdmissionStage::Complete)) && matches!(sent, Ok(AdmissionStage::Complete)) {
                completed = true;
                break;
            }
            let staged = joining.store.admission_history_checkpoint().await.unwrap();
            let cursor = staged.as_ref().map_or(joining.status().applied_count as usize, |saved| saved.records.len());
            let phase = match joining.admission_stage() {
                AdmissionStage::AwaitingApproval => 0,
                AdmissionStage::Approved => 1,
                AdmissionStage::PendingMaterials => 2,
                AdmissionStage::Complete => 3,
            };
            eprintln!("PAGED_ADMISSION_RETRY contact={contact} cursor={cursor} phase={phase} received={received:?} sent={sent:?}");
            assert!((cursor, phase) > previous, "each explicit contact must retain new verified records or finish material delivery");
            previous = (cursor, phase);
            if contact == 0 {
                if staged.as_ref().is_some_and(|saved| !saved.complete) {
                    assert_eq!(joining.status().applied_count, 0, "a partial prefix never publishes membership");
                }
                let binding = staged.as_ref().unwrap().binding;
                joining.close().await;
                joining = NativeNetwork::open(&path).await.unwrap();
                let reopened = joining.store.admission_history_checkpoint().await.unwrap().unwrap();
                assert_eq!(reopened.binding, binding);
                assert_eq!(reopened.records.len(), cursor);
            }
        }
        assert!(completed, "same precode request must finish after bounded explicit contacts");
        assert_eq!(admin.precode_usage(&code).await.unwrap(), 1);
        let redemptions = admin.precode_redemptions(&code).await.unwrap();
        assert_eq!(redemptions.len(), 1, "contacts must reuse the original redemption and authorization");
        assert_eq!(redemptions[0].stage, AdmissionStage::Complete);
        assert_eq!(joining.status().applied_count, crate::management_record::MAX_PAGE_RECORDS as i64 + 2);
        assert_eq!(joining.store.applied_records().await.unwrap(), admin.store.applied_records().await.unwrap());
        provider.close().await;
        joining.close().await;
        let reopened = NativeNetwork::open(&path).await.unwrap();
        assert_eq!(reopened.admission_stage(), AdmissionStage::Complete);
        assert_eq!(reopened.status().applied_count, admin.status().applied_count);
        reopened.close().await;
        admin.close().await;
    }
}
