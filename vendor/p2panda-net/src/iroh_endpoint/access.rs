// SPDX-License-Identifier: MIT OR Apache-2.0

//! Private ownership variants share the existing endpoint and connection actors.
//! Only the independently typed public wrapper can query a restricted endpoint.
use iroh::{
    EndpointAddr,
    endpoint::{
        AlpnError, ConnectOptions, ConnectWithOptsError, ConnectingError, Connection,
        ConnectionError,
    },
    protocol::{AcceptError, DynProtocolHandler},
};

#[derive(Clone, Debug)]
pub enum EndpointAccess {
    Ordinary(iroh::Endpoint),
    Restricted(iroh::endpoint::RestrictedEndpoint),
}
impl EndpointAccess {
    pub fn addr(&self) -> EndpointAddr {
        match self {
            Self::Ordinary(endpoint) => endpoint.addr(),
            Self::Restricted(endpoint) => endpoint.addr(),
        }
    }
    pub fn set_alpns(&self, alpns: Vec<Vec<u8>>) {
        match self {
            Self::Ordinary(endpoint) => endpoint.set_alpns(alpns),
            Self::Restricted(endpoint) => endpoint.set_alpns(alpns),
        }
    }
    pub async fn close(&self) {
        match self {
            Self::Ordinary(endpoint) => endpoint.close().await,
            Self::Restricted(endpoint) => endpoint.close().await,
        }
    }
    pub async fn accept(&self) -> Option<IncomingAccess> {
        match self {
            Self::Ordinary(endpoint) => endpoint.accept().await.map(IncomingAccess::Ordinary),
            Self::Restricted(endpoint) => endpoint.accept().await.map(IncomingAccess::Restricted),
        }
    }
    pub async fn connect_with_opts(
        &self,
        address: EndpointAddr,
        alpn: &[u8],
        options: ConnectOptions,
    ) -> Result<ConnectingAccess, ConnectWithOptsError> {
        match self {
            Self::Ordinary(endpoint) => endpoint
                .connect_with_opts(address, alpn, options)
                .await
                .map(ConnectingAccess::Ordinary),
            Self::Restricted(endpoint) => endpoint
                .connect_with_opts(address, alpn, options)
                .await
                .map(ConnectingAccess::Restricted),
        }
    }
}
#[derive(Debug)]
pub enum ConnectingAccess {
    Ordinary(iroh::endpoint::Connecting),
    Restricted(iroh::endpoint::RestrictedConnecting),
}
impl ConnectingAccess {
    pub async fn finish(self) -> Result<Connection, ConnectingError> {
        match self {
            Self::Ordinary(connecting) => connecting.await,
            Self::Restricted(connecting) => connecting.await,
        }
    }
}
#[derive(Debug)]
pub enum IncomingAccess {
    Ordinary(iroh::endpoint::Incoming),
    Restricted(iroh::endpoint::RestrictedIncoming),
}
impl IncomingAccess {
    pub fn accept(self) -> Result<AcceptingAccess, ConnectionError> {
        match self {
            Self::Ordinary(incoming) => incoming.accept().map(AcceptingAccess::Ordinary),
            Self::Restricted(incoming) => incoming.accept().map(AcceptingAccess::Restricted),
        }
    }
}
#[derive(Debug)]
pub enum AcceptingAccess {
    Ordinary(iroh::endpoint::Accepting),
    Restricted(iroh::endpoint::RestrictedAccepting),
}
impl AcceptingAccess {
    pub async fn alpn(&mut self) -> Result<Vec<u8>, AlpnError> {
        match self {
            Self::Ordinary(accepting) => accepting.alpn().await,
            Self::Restricted(accepting) => accepting.alpn().await,
        }
    }
    pub async fn hand_off(
        self,
        handler: &dyn DynProtocolHandler,
    ) -> Result<Connection, AcceptError> {
        match self {
            Self::Ordinary(accepting) => handler.on_accepting(accepting).await,
            // The ordinary handler cannot receive a pre-admission Accepting.
            Self::Restricted(accepting) => accepting.await.map_err(AcceptError::from_err),
        }
    }
}
