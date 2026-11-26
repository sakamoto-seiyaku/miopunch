use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use iroh::EndpointAddr;
use tokio::sync::{Mutex, Notify};

use crate::{
    admission_rotation::{RotationError, RotationProgress, RotationStage},
    control_protocol::AuthenticatedControl,
    daily_management::ManagementSession,
    genesis_recovery::GenesisCredential,
    native_client::{ClientError, NativeClient},
    native_network::NativeNetwork,
    recovery_card::RecoveryCard,
    resources::{ContactClass, ResourceEndpoint},
    service_protocol::next_member_sync_peer,
};

const CONFIRMATION_WINDOW: Duration = Duration::from_secs(60);

/// A live, device-local confirmation can authorize only the saved rotation ID.
/// Closing the client drops its elevation; the worker can still distribute
/// material and verified completion using the durable proposal after reopen.
#[derive(Default)]
pub(crate) struct RotationConfirmation {
    pending: Mutex<Option<([u8; 16], ManagementSession)>>,
    wake: Notify,
}

impl NativeClient {
    pub async fn rotation_progress(&self) -> Result<Option<RotationProgress>, RotationError> {
        let client_runtime = self.runtime();
        client_runtime.node.lock().await.rotation_progress().await
    }

    /// Start a rotation explicitly confirmed for this network. With no recovery
    /// card, use the verified credential already held by this device.
    pub async fn begin_admission_rotation(
        &self,
        card: Option<&RecoveryCard>,
    ) -> Result<RotationProgress, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let mut node = client_runtime.node.lock().await;
        let mut session = confirmed_session(&node, card)?;
        let progress = node.begin_admission_rotation(&mut session).await?;
        *client_runtime.rotation_confirmation.pending.lock().await = Some((progress.id, session));
        client_runtime.rotation_confirmation.wake.notify_one();
        Ok(progress)
    }

    /// Renew confirmation for the same pending rotation. The worker commits
    /// only after all still-retained members have acknowledged durably.
    pub async fn confirm_admission_rotation(
        &self,
        id: [u8; 16],
        card: Option<&RecoveryCard>,
    ) -> Result<RotationProgress, ClientError> {
        let client_runtime = self.runtime();
        client_runtime.require_ready()?;
        let node = client_runtime.node.lock().await;
        let progress = node
            .rotation_progress()
            .await?
            .ok_or(RotationError::Rejected)?;
        if progress.id != id || progress.initiator != node.status().node_id {
            return Err(RotationError::Rejected.into());
        }
        if progress.stage == RotationStage::Distributing {
            let session = confirmed_session(&node, card)?;
            *client_runtime.rotation_confirmation.pending.lock().await = Some((id, session));
            client_runtime.rotation_confirmation.wake.notify_one();
        }
        Ok(progress)
    }
}

fn confirmed_session(
    node: &NativeNetwork,
    card: Option<&RecoveryCard>,
) -> Result<ManagementSession, ClientError> {
    let credential = match card {
        Some(card) => GenesisCredential::from_saved(card.credential().0, card.anchor().genesis),
        None => node.saved_genesis_credential()?,
    };
    let mut session = node.begin_management_session();
    node.elevate_genesis(&mut session, credential, CONFIRMATION_WINDOW)?;
    Ok(session)
}

pub(crate) async fn maintain(
    node: Arc<Mutex<NativeNetwork>>,
    resources: Arc<ResourceEndpoint>,
    recovery_saved: Arc<AtomicBool>,
    replacement_ready: Arc<AtomicBool>,
) {
    let mut ticks = rotation_ticks(&resources);
    let mut changes = node.lock().await.store.changes.subscribe();
    let mut rotation = None;
    let mut completed = BTreeSet::new();
    let mut last = None;
    loop {
        tokio::select! {
            _ = ticks.tick() => {},
            _ = changes.changed() => {},
        }
        if !recovery_saved.load(Ordering::Acquire) || !replacement_ready.load(Ordering::Acquire) {
            continue;
        }
        let (progress, candidates) = {
            let mut node = node.lock().await;
            let Ok(Some((progress, candidates))) = node.rotation_delivery_plan().await else {
                continue;
            };
            (progress, candidates)
        };
        if rotation != Some(progress.id) {
            rotation = Some(progress.id);
            completed.clear();
            last = None;
        }
        let peers = candidates.keys().copied().filter(|peer| {
            !completed.contains(peer) && resources.contact_ready(*peer, ContactClass::Rotation)
        });
        let Some(peer) = next_member_sync_peer(peers, last) else {
            continue;
        };
        last = Some(peer);
        let alpn = candidates[&peer];
        let Ok((connection, attempt)) = resources
            .connect_control_attempt(EndpointAddr::new(peer), alpn, ContactClass::Rotation)
            .await
        else {
            continue;
        };
        let result = NativeNetwork::deliver_current_rotation(
            &node,
            &connection,
            progress.id,
            progress.stage,
        )
        .await;
        connection.close(0u32.into(), b"rotation exchange ended");
        match result {
            Ok(delivered) => {
                attempt.succeeded();
                if delivered.stage == RotationStage::Complete {
                    completed.insert(peer);
                }
            }
            Err(error) => {
                attempt.failed(error.to_string());
            }
        }
    }
}

/// A durable ACK can authorize the switch while the delivery task is still
/// waiting for the peer's final transport confirmation.
pub(crate) async fn commit_when_ready(
    node: Arc<Mutex<NativeNetwork>>,
    resources: Arc<ResourceEndpoint>,
    confirmation: Arc<RotationConfirmation>,
) {
    let mut ticks = rotation_ticks(&resources);
    let mut changes = node.lock().await.store.changes.subscribe();
    loop {
        tokio::select! {
            _ = ticks.tick() => {},
            _ = changes.changed() => {},
            _ = confirmation.wake.notified() => {},
        }
        let mut node = node.lock().await;
        let mut pending = confirmation.pending.lock().await;
        let Some((id, session)) = pending.as_mut() else {
            continue;
        };
        if session.elevation_remaining().is_none() {
            *pending = None;
            continue;
        }
        let Ok(Some(progress)) = node.rotation_progress().await else {
            continue;
        };
        if *id != progress.id
            || progress.stage == RotationStage::Complete
            || progress.retained.len() == progress.acknowledged.len()
                && node.finish_admission_rotation(session).await.is_ok()
        {
            *pending = None;
        }
    }
}

fn rotation_ticks(resources: &ResourceEndpoint) -> tokio::time::Interval {
    let interval = Duration::from_millis(resources.maintenance_policy().exploration_interval_ms);
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticks
}
