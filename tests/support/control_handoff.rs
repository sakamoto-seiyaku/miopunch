//! Composes the real admission gate with an externally prepared member path.
use iroh::endpoint::{
    AdmissionOutcome, Connection, ControlConnection, ControlHandle, EndpointAdmission, PathId,
};
use miopunch_native_probe::{admission_gate::AdmissionGate, control_session::ControlSession};
use std::io;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct PendingDataPolicy {
    pub gate: AdmissionGate,
    pub pending: mpsc::UnboundedSender<PendingControl>,
}
#[derive(Debug)]
pub struct PendingControl {
    pub control: ControlHandle,
    ready: oneshot::Sender<PathId>,
}
impl EndpointAdmission for PendingDataPolicy {
    async fn admit(&self, control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let AdmissionOutcome::ControlOnlyWithConfirmation(confirmation) =
            self.gate.admit(control).await?
        else {
            return Err(io::Error::other("expected authenticated control purpose"));
        };
        if !self.gate.allows_member_path_upgrade(control) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "current membership required",
            ));
        }
        let (ready, receive) = oneshot::channel();
        self.pending
            .send(PendingControl {
                control: control.control_handle(),
                ready,
            })
            .map_err(|_| io::Error::other("path preparer dropped"))?;
        let path = receive
            .await
            .map_err(|_| io::Error::other("path preparation canceled"))?;
        let permit = control.qualify_data_path(path, vec![]).await?;
        Ok(AdmissionOutcome::AcceptWithConfirmation {
            permit,
            confirmation,
        })
    }
    fn is_current(&self, control: &ControlConnection) -> bool {
        self.gate.allows_member_path_upgrade(control)
    }
    fn business_released(&self, connection: &Connection) {
        self.gate.business_released(connection)
    }
}

/// Abort only the pending connection task owned by this handoff.
pub struct OwnedConnectionTask(pub tokio::task::JoinHandle<io::Result<Connection>>);
impl Drop for OwnedConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl PendingControl {
    pub fn session(self, task: tokio::task::JoinHandle<io::Result<Connection>>) -> ControlSession {
        let task = OwnedConnectionTask(task);
        ControlSession::with_data_handoff(self.control, move |control| async move {
            let paths = control.paths();
            let path = paths
                .iter()
                .find(|path| path.is_selected() && path.is_ip())
                .ok_or_else(|| io::Error::other("no selected member IP path"))?
                .id();
            self.ready
                .send(path)
                .map_err(|_| io::Error::other("pending admission dropped"))?;
            let mut task = task;
            (&mut task.0).await.map_err(io::Error::other)?
        })
    }
}
