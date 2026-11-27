//! Monitoring of networking interfaces and route changes.

use n0_error::{e, stack_error};
use n0_future::task::{self, AbortOnDropHandle};
use n0_watcher::Watchable;
use tokio::sync::{mpsc, oneshot};

mod actor;
#[cfg(target_os = "android")]
mod android;
#[cfg(bsd)]
mod bsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(posix_minimal)]
mod posix_minimal;
#[cfg(wasm_browser)]
mod wasm_browser;
#[cfg(target_os = "windows")]
mod windows;

use self::actor::{Actor, ActorMessage};
pub use crate::interfaces::State;

/// Monitors networking interface and route changes.
#[derive(Debug)]
pub struct Monitor {
    /// Task handle for the monitor task.
    _handle: AbortOnDropHandle<()>,
    actor_tx: mpsc::Sender<ActorMessage>,
    interface_state: Watchable<State>,
    network_activity: Watchable<u64>,
}

#[stack_error(derive, add_meta, from_sources)]
#[non_exhaustive]
pub enum Error {
    #[error("channel closed")]
    ChannelClosed {},
    #[error("actor error")]
    Actor { source: actor::Error },
}

impl<T> From<mpsc::error::SendError<T>> for Error {
    fn from(_value: mpsc::error::SendError<T>) -> Self {
        e!(Error::ChannelClosed)
    }
}

impl From<oneshot::error::RecvError> for Error {
    fn from(_value: oneshot::error::RecvError) -> Self {
        e!(Error::ChannelClosed)
    }
}

impl Monitor {
    /// Create a new monitor.
    pub async fn new() -> Result<Self, Error> {
        let actor = Actor::new().await?;
        let actor_tx = actor.subscribe();
        let interface_state = actor.state().clone();
        let network_activity = actor.network_activity().clone();

        let handle = task::spawn(async move {
            actor.run().await;
        });

        Ok(Monitor {
            _handle: AbortOnDropHandle::new(handle),
            actor_tx,
            interface_state,
            network_activity,
        })
    }

    /// Subscribe to network changes.
    pub fn interface_state(&self) -> n0_watcher::Direct<State> {
        self.interface_state.watch()
    }

    /// Subscribe to route activity that leaves the interface snapshot unchanged.
    pub fn network_activity(&self) -> n0_watcher::Direct<u64> {
        self.network_activity.watch()
    }

    /// Potential change detected outside
    pub async fn network_change(&self) -> Result<(), Error> {
        let (reply, received) = oneshot::channel();
        self.actor_tx
            .send(ActorMessage::NetworkChange(reply))
            .await?;
        received.await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use n0_watcher::Watcher as _;

    use super::*;

    #[tokio::test]
    async fn test_smoke_monitor() {
        let mon = Monitor::new().await.unwrap();
        let mut sub = mon.interface_state();

        let current = sub.get();
        println!("current state: {current}");
    }
}
