//! Online direct carrier candidates travel over the already admitted QUIC connection.

use std::io;

use futures_util::{StreamExt, future::join_all};
use iroh_base::CustomAddr;
use tokio::{
    sync::{mpsc, watch},
    time::{self, Duration},
};

use super::{Connection, RecvStream, TransportAddr};
use crate::endpoint::Endpoint;

const PREFACE: &[u8] = b"iroh/member-direct/1\0";
const MAX_CANDIDATES: usize = 2;
const MAX_ADDRESS: usize = 32;
const EXCHANGE_LIMIT: Duration = Duration::from_secs(6);
const VALIDATION_LIMIT: Duration = Duration::from_secs(6);

pub(in crate::endpoint) fn improve_direct_path(
    endpoint: Endpoint,
    connection: Connection,
    activity: watch::Receiver<bool>,
    transport: u64,
) -> mpsc::Sender<CustomAddr> {
    let (sender, candidates) = mpsc::channel(MAX_CANDIDATES);
    tokio::spawn(improve(
        endpoint, connection, activity, transport, candidates,
    ));
    sender
}

async fn improve(
    endpoint: Endpoint,
    connection: Connection,
    mut activity: watch::Receiver<bool>,
    transport: u64,
    mut candidates: mpsc::Receiver<CustomAddr>,
) {
    let addresses = || {
        endpoint
            .inner
            .local_addr()
            .into_iter()
            .filter_map(|address| {
                let TransportAddr::Custom(address) = address.into() else {
                    return None;
                };
                (address.id() == transport).then_some(address)
            })
    };
    if addresses().next().is_none() {
        return;
    }
    let policy = endpoint
        .inner
        .static_config
        .admission
        .as_ref()
        .expect("member policy");
    let permitted = |address: &CustomAddr| {
        address.id() == transport
            && endpoint
                .inner
                .static_config
                .relay_resource_policy
                .allows_custom(address)
            && policy.permits_direct_candidate(address)
    };
    let mut offer_activity = activity.clone();
    let offers = async {
        // A peer's offer also activates this exchange before the application
        // accepts an incoming business stream. Member business uses bidi streams.
        let incoming = connection.accept_uni();
        tokio::pin!(incoming);
        let stream = tokio::select! {
            result = &mut incoming => Some(result.map_err(io::Error::other)?),
            result = offer_activity.wait_for(|active| *active) => {
                result.map_err(io::Error::other)?;
                None
            }
        };
        let remote = time::timeout(EXCHANGE_LIMIT, async {
            let send = async {
                let mut offer = PREFACE.to_vec();
                for address in addresses().filter(&permitted).take(MAX_CANDIDATES) {
                    let encoded = address.to_vec();
                    if encoded.len() <= MAX_ADDRESS {
                        offer.push(encoded.len() as u8);
                        offer.extend_from_slice(&encoded);
                    }
                }
                let mut stream = connection.open_uni().await.map_err(io::Error::other)?;
                stream.write_all(&offer).await.map_err(io::Error::other)?;
                stream.finish().map_err(io::Error::other)
            };
            let receive = async {
                let stream = match stream {
                    Some(stream) => stream,
                    None => incoming.await.map_err(io::Error::other)?,
                };
                read_offer(stream).await
            };
            let (_, remote) = tokio::try_join!(send, receive)?;
            Ok::<_, io::Error>(remote)
        })
        .await??;
        if connection.side().is_client() {
            let attempts = remote
                .into_iter()
                .filter(&permitted)
                .map(|address| validate(&endpoint, &connection, address, &permitted));
            let _ = time::timeout(VALIDATION_LIMIT, join_all(attempts)).await;
        }
        Ok::<_, io::Error>(())
    };
    let known = async {
        if connection.side().is_server() {
            return;
        }
        while let Some(address) = candidates.recv().await {
            if activity.wait_for(|active| *active).await.is_err() {
                break;
            }
            if permitted(&address) {
                let _ = time::timeout(
                    VALIDATION_LIMIT,
                    validate(&endpoint, &connection, address, &permitted),
                )
                .await;
            }
        }
    };
    tokio::select! {
        _ = connection.closed() => {}
        _ = async { let _ = tokio::join!(offers, known); } => {}
    }
}

async fn read_offer(mut stream: RecvStream) -> io::Result<Vec<CustomAddr>> {
    let offer = stream
        .read_to_end(PREFACE.len() + MAX_CANDIDATES * (1 + MAX_ADDRESS))
        .await
        .map_err(io::Error::other)?;
    let mut body = offer
        .strip_prefix(PREFACE)
        .ok_or(io::ErrorKind::InvalidData)?;
    let mut addresses = Vec::new();
    while let Some((&length, rest)) = body.split_first() {
        let length = usize::from(length);
        if length > MAX_ADDRESS || addresses.len() == MAX_CANDIDATES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let (bytes, next) = rest
            .split_at_checked(length)
            .ok_or(io::ErrorKind::InvalidData)?;
        let address = CustomAddr::from_bytes(bytes).map_err(io::Error::other)?;
        addresses.push(address);
        body = next;
    }
    addresses.dedup();
    Ok(addresses)
}

async fn validate(
    endpoint: &Endpoint,
    connection: &Connection,
    address: CustomAddr,
    permitted: impl Fn(&CustomAddr) -> bool,
) -> io::Result<()> {
    let transport = address.id();
    let remote = TransportAddr::Custom(address.clone());
    let Some(candidate) = endpoint
        .inner
        .request_admitted_data_path(
            connection.remote_id(),
            connection.stable_id(),
            remote.clone(),
            true,
        )
        .await?
    else {
        return Ok(());
    };
    let mut paths = connection.paths_stream();
    let probe = loop {
        let paths = paths.next().await.ok_or(io::ErrorKind::NotConnected)?;
        if let Some(path) = paths
            .iter()
            .find(|path| path.id() == candidate.id() && path.remote_addr() == &remote)
        {
            break path.start_probe()?;
        }
    };
    while !probe.is_acknowledged().map_err(io::Error::other)? {
        time::sleep(Duration::from_millis(15)).await;
    }
    if permitted(&address)
        && endpoint
            .inner
            .qualifies_admission_data_path(
                connection.remote_id(),
                connection.stable_id(),
                &remote,
                &[transport],
            )
            .await
    {
        candidate.retain();
    }
    Ok(())
}
