//! The authenticated metadata and bounded control messages shared by the old
//! ordinary counterpart and the typed control interface. No admission decision
//! follows from this trait itself; the gate still binds every fact to one session.
use iroh::endpoint::{
    Connection, ConnectionError, ControlConnection, ControlExchange, ControlHandle,
    ExportKeyingMaterialError, PathList, Side, VarInt, WeakControlConnection,
};
use iroh_base::EndpointId;
use n0_future::time::Instant;
use std::{fmt::Debug, io, sync::Arc};

/// Authenticated session facts and complete control messages, never business I/O.
pub trait AuthenticatedControl: Debug + Send + Sync {
    fn remote_id(&self) -> EndpointId;
    fn stable_id(&self) -> usize;
    fn side(&self) -> Side;
    fn alpn(&self) -> &[u8];
    fn caller_deadline(&self) -> Option<Instant>;
    fn attempt_cancellation(&self) -> Option<tokio_util::sync::CancellationToken>;
    fn close_reason(&self) -> Option<ConnectionError>;
    fn close(&self, code: VarInt, reason: &[u8]);
    fn paths(&self) -> PathList<'_>;
    fn revocation_handle(&self) -> WeakControlConnection;
    fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError>;
    fn open_control_exchange(&self) -> impl Future<Output = io::Result<ControlExchange>> + Send;
    fn accept_control_exchange(&self) -> impl Future<Output = io::Result<ControlExchange>> + Send;
}

macro_rules! inherent_metadata {
    ($ty:ty) => {
        fn remote_id(&self) -> EndpointId {
            <$ty>::remote_id(self)
        }
        fn stable_id(&self) -> usize {
            <$ty>::stable_id(self)
        }
        fn side(&self) -> Side {
            <$ty>::side(self)
        }
        fn alpn(&self) -> &[u8] {
            <$ty>::alpn(self)
        }
        fn caller_deadline(&self) -> Option<Instant> {
            <$ty>::caller_deadline(self)
        }
        fn attempt_cancellation(&self) -> Option<tokio_util::sync::CancellationToken> {
            <$ty>::attempt_cancellation(self)
        }
        fn close_reason(&self) -> Option<ConnectionError> {
            <$ty>::close_reason(self)
        }
        fn close(&self, code: VarInt, reason: &[u8]) {
            <$ty>::close(self, code, reason)
        }
        fn paths(&self) -> PathList<'_> {
            <$ty>::paths(self)
        }
        fn export_keying_material(
            &self,
            output: &mut [u8],
            label: &[u8],
            context: &[u8],
        ) -> Result<(), ExportKeyingMaterialError> {
            <$ty>::export_keying_material(self, output, label, context)
        }
    };
}
impl AuthenticatedControl for Connection {
    inherent_metadata!(Connection);
    fn revocation_handle(&self) -> WeakControlConnection {
        self.weak_handle().into()
    }
    async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        let (send, recv) = self.open_bi().await.map_err(io::Error::other)?;
        Ok(ControlExchange::from_streams(send, recv))
    }
    async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        let (send, recv) = self.accept_bi().await.map_err(io::Error::other)?;
        Ok(ControlExchange::from_streams(send, recv))
    }
}
impl AuthenticatedControl for ControlConnection {
    inherent_metadata!(ControlHandle);
    fn revocation_handle(&self) -> WeakControlConnection {
        self.weak_handle()
    }
    async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        ControlHandle::open_control_exchange(self).await
    }
    async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        ControlHandle::accept_control_exchange(self).await
    }
}
impl AuthenticatedControl for ControlHandle {
    inherent_metadata!(ControlHandle);
    fn revocation_handle(&self) -> WeakControlConnection {
        self.weak_handle()
    }
    async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        ControlHandle::open_control_exchange(self).await
    }
    async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        ControlHandle::accept_control_exchange(self).await
    }
}
impl<T: AuthenticatedControl + ?Sized> AuthenticatedControl for Arc<T> {
    fn remote_id(&self) -> EndpointId {
        self.as_ref().remote_id()
    }
    fn stable_id(&self) -> usize {
        self.as_ref().stable_id()
    }
    fn side(&self) -> Side {
        self.as_ref().side()
    }
    fn alpn(&self) -> &[u8] {
        self.as_ref().alpn()
    }
    fn caller_deadline(&self) -> Option<Instant> {
        self.as_ref().caller_deadline()
    }
    fn attempt_cancellation(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.as_ref().attempt_cancellation()
    }
    fn close_reason(&self) -> Option<ConnectionError> {
        self.as_ref().close_reason()
    }
    fn close(&self, code: VarInt, reason: &[u8]) {
        self.as_ref().close(code, reason)
    }
    fn paths(&self) -> PathList<'_> {
        self.as_ref().paths()
    }
    fn revocation_handle(&self) -> WeakControlConnection {
        self.as_ref().revocation_handle()
    }
    fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError> {
        self.as_ref().export_keying_material(output, label, context)
    }
    async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        self.as_ref().open_control_exchange().await
    }
    async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        self.as_ref().accept_control_exchange().await
    }
}

/// Ownership for an admission exchange that may return a durable delivery handle.
/// Borrowed pre-admission control references deliberately do not implement this.
pub trait OwnedControl: AuthenticatedControl {
    fn retain_control(&self) -> ControlOwner;
}

#[derive(Clone, Debug)]
enum Owner {
    Ordinary(Connection),
    Restricted(Arc<ControlConnection>),
}

/// Retains an already-owned control exchange without exposing business methods.
/// There is no conversion or borrow of the underlying connection or streams.
#[derive(Clone, Debug)]
pub struct ControlOwner {
    inner: Owner,
}
impl ControlOwner {
    pub(crate) fn restricted_handle(&self) -> Option<ControlHandle> {
        match &self.inner {
            Owner::Restricted(c) => Some(c.control_handle()),
            Owner::Ordinary(_) => None,
        }
    }
    pub(crate) fn path_handle(
        &self,
        endpoint: &iroh::endpoint::RestrictedEndpoint,
    ) -> ControlHandle {
        match &self.inner {
            Owner::Restricted(c) => c.control_handle(),
            Owner::Ordinary(c) => endpoint.control_handle(c),
        }
    }
}
impl OwnedControl for Connection {
    fn retain_control(&self) -> ControlOwner {
        ControlOwner {
            inner: Owner::Ordinary(self.clone()),
        }
    }
}
impl OwnedControl for Arc<ControlConnection> {
    fn retain_control(&self) -> ControlOwner {
        ControlOwner {
            inner: Owner::Restricted(self.clone()),
        }
    }
}
impl OwnedControl for ControlOwner {
    fn retain_control(&self) -> ControlOwner {
        self.clone()
    }
}
macro_rules! owner_metadata {
    ($name:ident,$output:ty) => {
        fn $name(&self) -> $output {
            match &self.inner {
                Owner::Ordinary(c) => c.$name(),
                Owner::Restricted(c) => c.$name(),
            }
        }
    };
}
impl AuthenticatedControl for ControlOwner {
    owner_metadata!(remote_id, EndpointId);
    owner_metadata!(stable_id, usize);
    owner_metadata!(side, Side);
    owner_metadata!(alpn, &[u8]);
    owner_metadata!(caller_deadline, Option<Instant>);
    owner_metadata!(
        attempt_cancellation,
        Option<tokio_util::sync::CancellationToken>
    );
    owner_metadata!(close_reason, Option<ConnectionError>);
    owner_metadata!(paths, PathList<'_>);
    fn close(&self, code: VarInt, reason: &[u8]) {
        match &self.inner {
            Owner::Ordinary(c) => c.close(code, reason),
            Owner::Restricted(c) => c.close(code, reason),
        }
    }
    fn revocation_handle(&self) -> WeakControlConnection {
        match &self.inner {
            Owner::Ordinary(c) => c.revocation_handle(),
            Owner::Restricted(c) => c.revocation_handle(),
        }
    }
    fn export_keying_material(
        &self,
        output: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), ExportKeyingMaterialError> {
        match &self.inner {
            Owner::Ordinary(c) => c.export_keying_material(output, label, context),
            Owner::Restricted(c) => c.export_keying_material(output, label, context),
        }
    }
    async fn open_control_exchange(&self) -> io::Result<ControlExchange> {
        match &self.inner {
            Owner::Ordinary(c) => c.open_control_exchange().await,
            Owner::Restricted(c) => c.open_control_exchange().await,
        }
    }
    async fn accept_control_exchange(&self) -> io::Result<ControlExchange> {
        match &self.inner {
            Owner::Ordinary(c) => c.accept_control_exchange().await,
            Owner::Restricted(c) => c.accept_control_exchange().await,
        }
    }
}
