//! One-shot fault injection on the original connection's final control exchange.

use std::{io, sync::Mutex};
use tokio::sync::oneshot;

use super::{ControlExchange, MemberConnection};

/// Which side of the final bilateral confirmation to hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathCommitPoint {
    /// Both local proof and candidate exist. The last message is still unsent.
    BeforeConfirmation,
    /// The peer's confirmation was read. The local candidate is still tentative.
    AfterConfirmation,
}

/// Controls and observations for a single TCP or explicit enhanced UDP commit.
#[derive(Debug)]
pub struct PathCommitPause {
    /// Handle to the actual exchange, delivered when the requested point is reached.
    pub entered: oneshot::Receiver<noq::StreamControl>,
    /// Allow local processing to resume.
    pub resume: oneshot::Sender<()>,
    /// Resolves on successful local retention. A dropped sender means the commit exited early.
    pub retained: oneshot::Receiver<()>,
}

#[derive(Debug, Default)]
pub(in crate::endpoint) struct Gate(Mutex<Option<Hold>>);

#[derive(Debug)]
pub(in crate::endpoint) struct Hold {
    point: PathCommitPoint,
    wait: Option<(oneshot::Sender<noq::StreamControl>, oneshot::Receiver<()>)>,
    retained: oneshot::Sender<()>,
}

impl MemberConnection {
    /// Hold the next verified path at its actual commit point on this original.
    #[doc(hidden)]
    pub fn test_pause_path_commit(&self, point: PathCommitPoint) -> PathCommitPause {
        let (notify, entered) = oneshot::channel();
        let (resume, wait) = oneshot::channel();
        let (committed, retained) = oneshot::channel();
        let mut slot = self
            .0
            .connection
            .commit_gate()
            .0
            .lock()
            .expect("commit fixture");
        assert!(slot.is_none(), "commit pause already armed");
        *slot = Some(Hold {
            point,
            wait: Some((notify, wait)),
            retained: committed,
        });
        PathCommitPause {
            entered,
            resume,
            retained,
        }
    }
}

impl Gate {
    pub(in crate::endpoint) fn take(&self) -> Option<Hold> {
        self.0.lock().expect("commit fixture").take()
    }
}

impl Hold {
    pub(super) async fn pause(
        &mut self,
        point: PathCommitPoint,
        exchange: &ControlExchange,
    ) -> io::Result<()> {
        if point == self.point {
            let (notify, wait) = self.wait.take().expect("one commit checkpoint");
            let _ = notify.send(exchange.send_control().await?);
            let _ = wait.await;
        }
        Ok(())
    }

    pub(super) fn retained(self) {
        let _ = self.retained.send(());
    }
}
