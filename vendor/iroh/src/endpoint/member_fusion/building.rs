//! Positions count active peers, shared by every carrier on the endpoint.
use super::EndpointId;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::super::MEMBER_BUILD_LIMIT;

#[derive(Debug)]
pub(crate) struct Building {
    pub(crate) physical: super::physical::Pool,
    positions: Arc<Semaphore>,
    peers: Mutex<HashMap<EndpointId, Weak<Position>>>,
    changed: watch::Sender<()>,
}
impl Default for Building {
    fn default() -> Self {
        let changed = watch::Sender::default();
        Self {
            physical: super::physical::Pool::new(changed.clone()),
            positions: Arc::new(Semaphore::new(MEMBER_BUILD_LIMIT)),
            peers: Mutex::default(),
            changed,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct BuildPermit {
    _position: Arc<Position>,
}
#[derive(Debug)]
struct Position {
    _permit: OwnedSemaphorePermit,
    changed: watch::Sender<()>,
}
impl Drop for Position {
    fn drop(&mut self) {
        self.changed.send_modify(|_| {});
    }
}
impl Building {
    #[cfg(feature = "miopunch-test-hooks")]
    pub(crate) fn active_peers(&self) -> Vec<EndpointId> {
        self.peers
            .lock()
            .expect("building peers")
            .iter()
            .filter_map(|(peer, permit)| (permit.strong_count() != 0).then_some(*peer))
            .collect()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
    fn existing(&self, peer: EndpointId) -> Option<BuildPermit> {
        self.peers
            .lock()
            .expect("building peers")
            .get(&peer)
            .and_then(Weak::upgrade)
            .map(|position| BuildPermit {
                _position: position,
            })
    }
    fn install(&self, peer: EndpointId, permit: OwnedSemaphorePermit) -> BuildPermit {
        let mut peers = self.peers.lock().expect("building peers");
        if let Some(existing) = peers.get(&peer).and_then(Weak::upgrade) {
            return BuildPermit {
                _position: existing,
            };
        }
        peers.retain(|_, position| position.strong_count() != 0);
        let position = Arc::new(Position {
            _permit: permit,
            changed: self.changed.clone(),
        });
        peers.insert(peer, Arc::downgrade(&position));
        self.changed.send_modify(|_| {});
        BuildPermit {
            _position: position,
        }
    }
    pub(crate) fn try_acquire(&self, peer: EndpointId) -> Option<BuildPermit> {
        self.existing(peer).or_else(|| {
            self.positions
                .clone()
                .try_acquire_owned()
                .ok()
                .map(|permit| self.install(peer, permit))
        })
    }
    pub(crate) async fn acquire(&self, peer: EndpointId) -> BuildPermit {
        let mut changed = self.changed.subscribe();
        loop {
            if let Some(existing) = self.existing(peer) {
                return existing;
            }
            tokio::select! {
                permit=self.positions.clone().acquire_owned() => return self.install(peer,permit.expect("building positions stay open")),
                _=changed.changed() => {},
            }
        }
    }
}
