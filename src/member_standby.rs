//! Pending ordinary-member contacts owned by the host's existing maintenance loop.
use std::{collections::BTreeSet, sync::Arc, time::Duration};

use iroh::{EndpointAddr, EndpointId};
use tokio::{task::JoinSet, time::Instant};

use crate::{
    admission_gate::AdmissionGate,
    resources::{ContactClass, ResourceEndpoint},
};

pub(crate) struct MemberStandby {
    tasks: JoinSet<EndpointId>,
    pending: BTreeSet<EndpointId>,
    last: Option<EndpointId>,
    next: Instant,
}

impl MemberStandby {
    pub(crate) fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            pending: BTreeSet::new(),
            last: None,
            next: Instant::now(),
        }
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

    pub(crate) fn schedule(
        &mut self,
        resources: &Arc<ResourceEndpoint>,
        gate: &AdmissionGate,
        available: usize,
    ) {
        let policy = resources.maintenance_policy();
        if Instant::now() < self.next || policy.max_standby_member_connections == 0 {
            return;
        }
        self.next = Instant::now() + Duration::from_millis(policy.exploration_interval_ms);
        let live = gate.live_member_connections();
        let mut peers = gate.member_contact_peers();
        peers.sort_by_key(|id| (self.last.is_some_and(|last| *id <= last), *id));
        for id in peers {
            if self.pending.len() >= available {
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
