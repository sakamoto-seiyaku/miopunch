//! Real restricted endpoints for transport tests without an application admission protocol.
use iroh::{
    EndpointAddr,
    endpoint::{
        AdmissionOutcome, Builder, Connection, ControlConfirmation, ControlConnection,
        ControlHandle, EndpointAdmission, PathId, RestrictedEndpoint,
    },
};
use miopunch_native_probe::control_session::ControlSession;
use std::{io, sync::Arc};
use tokio::sync::{Mutex, mpsc, oneshot};
type Pending = (ControlHandle, oneshot::Sender<PathId>);
#[derive(Debug, Clone)]
struct PathPolicy {
    only: bool,
    pending: mpsc::UnboundedSender<Pending>,
}
impl EndpointAdmission for PathPolicy {
    async fn admit(&self, control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let confirmation = ControlConfirmation::new(async { Ok(()) });
        if self.only {
            return Ok(AdmissionOutcome::ControlOnlyWithConfirmation(confirmation));
        }
        let (ready, receive) = oneshot::channel();
        self.pending
            .send((control.control_handle(), ready))
            .map_err(|_| io::Error::other("path preparer closed"))?;
        let path = receive
            .await
            .map_err(|_| io::Error::other("path preparation canceled"))?;
        let permit = control.qualify_data_path(path, vec![]).await?;
        Ok(AdmissionOutcome::AcceptWithConfirmation {
            permit,
            confirmation,
        })
    }
    fn is_current(&self, _: &ControlConnection) -> bool {
        true
    }
}
struct ConnectionTask(tokio::task::JoinHandle<io::Result<Connection>>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Clone)]
pub struct PathEndpoint {
    inner: RestrictedEndpoint,
    only: bool,
    pending: Arc<Mutex<mpsc::UnboundedReceiver<Pending>>>,
}
impl std::ops::Deref for PathEndpoint {
    type Target = RestrictedEndpoint;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl PathEndpoint {
    pub async fn bind(builder: Builder, only: bool) -> Self {
        let (pending, receive) = mpsc::unbounded_channel();
        let inner = builder
            .restricted_application(PathPolicy { only, pending })
            .bind()
            .await
            .unwrap();
        Self {
            inner,
            only,
            pending: Arc::new(Mutex::new(receive)),
        }
    }
    pub async fn control(
        &self,
        target: Option<EndpointAddr>,
        alpn: &[u8],
    ) -> io::Result<ControlSession> {
        if self.only {
            let owner = match target {
                Some(target) => self
                    .inner
                    .connect_control(target, alpn)
                    .await
                    .map_err(io::Error::other)?,
                None => self
                    .inner
                    .accept()
                    .await
                    .unwrap()
                    .accept_control()
                    .map_err(io::Error::other)?
                    .await
                    .map_err(io::Error::other)?,
            };
            return Ok(ControlSession::from_control_connection(owner));
        }
        let endpoint = self.inner.clone();
        let alpn = alpn.to_vec();
        let mut task = ConnectionTask(tokio::spawn(async move {
            match target {
                Some(target) => endpoint
                    .connect(target, &alpn)
                    .await
                    .map_err(io::Error::other),
                None => endpoint
                    .accept()
                    .await
                    .unwrap()
                    .await
                    .map_err(io::Error::other),
            }
        }));
        let mut receive = self.pending.lock().await;
        let (control, ready) = tokio::select! {biased;
            result=&mut task.0=>{let _=result.map_err(io::Error::other)??;return Err(io::Error::other("business arrived before path preparation"));},
            pending=receive.recv()=>pending.ok_or_else(||io::Error::other("pending control closed"))?,
        };
        Ok(ControlSession::with_data_handoff(
            control,
            move |control| async move {
                let paths = control.paths();
                let path = paths
                    .iter()
                    .find(|path| path.is_selected() && path.is_ip())
                    .ok_or_else(|| io::Error::other("no selected IP path"))?
                    .id();
                ready
                    .send(path)
                    .map_err(|_| io::Error::other("Core admission canceled"))?;
                let mut task = task;
                (&mut task.0).await.map_err(io::Error::other)?
            },
        ))
    }
}
