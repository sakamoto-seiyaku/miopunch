use iroh::endpoint::{
    Builder, ConnectingError, Connection, ControlAccepting, ControlConnection, EndpointAdmission,
    RestrictedAccepting, RestrictedBuilder, RestrictedIncoming, WeakControlConnection,
};
use std::future::{Future, IntoFuture};

pub fn configure(builder: Builder, policy: impl EndpointAdmission + 'static) -> RestrictedBuilder {
    builder.restricted(policy)
}

pub fn accept(
    incoming: RestrictedIncoming,
) -> impl Future<Output = Result<Connection, ConnectingError>> {
    incoming.into_future()
}

pub fn business(
    accepting: RestrictedAccepting,
) -> impl Future<Output = Result<Connection, ConnectingError>> {
    accepting.into_future()
}

pub fn control(
    accepting: ControlAccepting,
) -> impl Future<Output = Result<ControlConnection, ConnectingError>> {
    accepting.into_future()
}

pub fn revocation(control: &ControlConnection) -> WeakControlConnection {
    control.weak_handle()
}
