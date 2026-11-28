//! Pending ordinary-member contacts owned by the host's existing maintenance loop.
use std::{collections::BTreeSet, sync::Arc, time::Duration};

use iroh::{EndpointAddr, EndpointId};
use tokio::{task::JoinSet, time::Instant};

use crate::{
    admission_gate::AdmissionGate,
    maintenance_policy::MEMBER_BUILD_LIMIT,
    resources::{ContactClass, ResourceEndpoint},
};

pub(crate) struct MemberStandby {
    tasks: JoinSet<EndpointId>,
    pending: BTreeSet<EndpointId>,
    last: Option<EndpointId>,
    next: Option<Instant>,
}

impl MemberStandby {
    pub(crate) fn new(resources: &ResourceEndpoint) -> Self {
        Self {
            tasks: JoinSet::new(),
            pending: BTreeSet::new(),
            last: None,
            next: resources.member_fusion.as_ref().map(|_| Instant::now()),
        }
    }

    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.next
    }

    pub(crate) fn defer_until(&mut self, at: Instant) {
        self.next = self.next.map(|next| next.max(at));
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) async fn completed(&mut self) {
        if let Some(Ok(id)) = self.tasks.join_next().await {
            self.pending.remove(&id);
        }
    }

    pub(crate) async fn stop(&mut self) {
        self.tasks.shutdown().await;
        self.pending.clear();
    }

    pub(crate) fn schedule(&mut self, resources: &Arc<ResourceEndpoint>, gate: &AdmissionGate) {
        let now = Instant::now();
        let Some(next) = &mut self.next else { return };
        if now < *next {
            return;
        }
        let policy = resources.maintenance_policy();
        *next = now + Duration::from_millis(policy.exploration_interval_ms);
        if policy.standby_member_limit == Some(0) {
            return;
        }
        let live = gate.live_member_connections();
        let mut peers = gate.member_contact_peers();
        peers.sort_by_key(|id| (self.last.is_some_and(|last| *id <= last), *id));
        for id in peers {
            if self.pending.len() >= MEMBER_BUILD_LIMIT {
                break;
            }
            if self.pending.contains(&id)
                || live.iter().any(|connection| {
                    connection.remote_id() == id
                        && connection.alpn() == crate::native_network::FUSED_MEMBER_ALPN
                })
                || !resources.contact_ready(id, ContactClass::Standby)
            {
                continue;
            }
            self.last = Some(id);
            self.pending.insert(id);
            let resources = resources.clone();
            self.tasks.spawn(async move {
                let deadline = Instant::now()
                    + Duration::from_millis(resources.maintenance_policy().contact_timeout_ms);
                if let Err(error) = resources
                    .connect_member_background(EndpointAddr::new(id), deadline)
                    .await
                {
                    eprintln!("MEMBER_STANDBY_CONTACT_FAILED peer={id} error={error}");
                }
                id
            });
        }
    }
}
