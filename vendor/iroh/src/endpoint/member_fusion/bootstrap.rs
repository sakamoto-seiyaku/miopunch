//! Concurrent paths on the protected first-contact connection.

use std::io;

use futures_util::{StreamExt, stream::FuturesUnordered};
use iroh_relay::ObservationFamily;

use super::{
    ControlConnection, ControlExchange, ControlHandle, ControlMessageFormat, DataPathPermit,
    MEMBER_DATA_UNAVAILABLE, direct, time,
};

enum BootstrapPath {
    Tcp(ObservationFamily),
    Udp(ObservationFamily),
}

// Both endpoints open or accept these protected exchanges in wire order.
const EXCHANGES: [(u8, BootstrapPath); 4] = [
    (1, BootstrapPath::Tcp(ObservationFamily::Ipv4)),
    (2, BootstrapPath::Udp(ObservationFamily::Ipv4)),
    (3, BootstrapPath::Udp(ObservationFamily::Ipv6)),
    (4, BootstrapPath::Tcp(ObservationFamily::Ipv6)),
];

/// Only these already allocated internal exchanges outlive application-control
/// retirement. The worker remains bound to the original attempt and connection.
pub(super) async fn qualify(
    connection: &ControlConnection,
    custom: Vec<u64>,
    ip_data_enabled: bool,
) -> io::Result<DataPathPermit> {
    let control = connection.member_bootstrap_handle();
    control.prepare_member_udp().await?;
    let mut exchanges = Vec::with_capacity(EXCHANGES.len());
    for (tag, path) in EXCHANGES {
        exchanges.push((path, exchange(&control, tag).await?));
    }
    let deadline = control
        .caller_deadline()
        .expect("restricted admission deadline");
    let cancel = control.attempt_cancellation().unwrap_or_default();
    let (completed, mut results) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let qualify_paths = async {
            let mut candidates = FuturesUnordered::new();
            let control = &control;
            for (path, exchange) in exchanges {
                let custom = custom.clone();
                candidates.push(async move {
                    match path {
                        BootstrapPath::Tcp(family) => tcp(control, exchange, custom, family).await,
                        BootstrapPath::Udp(family) => {
                            udp(control, exchange, custom, family, ip_data_enabled).await
                        }
                    }
                });
            }
            while let Some(result) = candidates.next().await {
                // Keep polling the original connection's remaining paths after
                // the caller takes its first permit and drops the receiver.
                let _ = completed.send(result);
            }
        };
        tokio::select! { biased;
            _ = control.closed() => {},
            _ = cancel.cancelled() => {},
            _ = time::sleep_until(deadline.into()) => {},
            _ = qualify_paths => {},
        }
    });
    while let Some(result) = results.recv().await {
        match result {
            Ok(permit) => return Ok(permit),
            Err(error) => tracing::debug!(stable_id = connection.stable_id(), %error,
                "member bootstrap candidate did not qualify"),
        }
    }
    connection.close(0u32.into(), MEMBER_DATA_UNAVAILABLE);
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "no permitted member data path",
    ))
}

async fn exchange(control: &ControlHandle, tag: u8) -> io::Result<ControlExchange> {
    let format = ControlMessageFormat::Fixed { length: 1 };
    if control.side().is_client() {
        let exchange = control.open_control_exchange().await?;
        exchange.send_message(format, &[tag]).await?;
        Ok(exchange)
    } else {
        let exchange = control.accept_control_exchange().await?;
        if exchange.recv_message(format).await? != [tag] {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(exchange)
    }
}

async fn tcp(
    control: &ControlHandle,
    exchange: ControlExchange,
    custom: Vec<u64>,
    family: ObservationFamily,
) -> io::Result<DataPathPermit> {
    let policy = control
        .endpoint
        .inner
        .static_config
        .admission
        .as_ref()
        .expect("member policy");
    let local = policy.member_direct().and_then(|direct| {
        if !custom.contains(&direct.transport_id()) || direct.traversal().is_none() {
            return None;
        }
        let binding = direct.bindings().into_iter().find(|binding| family.matches(*binding))?;
        Some((direct, binding))
    });
    let format = ControlMessageFormat::Fixed { length: 1 };
    exchange.send_message(format, &[u8::from(local.is_some())]).await?;
    let (direct, configured) = match exchange.recv_message(format).await?.as_slice() {
        [1] => local.ok_or(io::ErrorKind::Unsupported)?,
        [0] => return Err(io::ErrorKind::Unsupported.into()),
        _ => return Err(io::ErrorKind::InvalidData.into()),
    };
    let deadline = control.caller_deadline().expect("restricted admission deadline").into();
    let building = &policy.member_fusion().expect("member fusion").building;
    let observers = direct.observers();
    let binding = if !observers.is_empty() {
        let (binding, destination) = super::physical::observer_binding(
                &control.endpoint.inner,
                family,
                observers.into_iter().map(|url| (url, 3478)),
                Some(configured),
                deadline,
            )
            .await?;
        Some((binding, control.endpoint.binding_route_generation(binding, destination)?))
    } else {
        None
    };
    let rendezvous = async { loop {
        let mut changed = building.subscribe();
        let permit = match &binding {
            Some((binding, validity)) => {
                let scope = control.endpoint.inner.punching_scope(*binding, true)?;
                building
                    .physical
                    .try_acquire(scope, control.remote_id())
                    .map(|permit| permit.with_binding(*binding, validity.clone()))
            }
            None => Some(super::physical::MemberPunching::untracked()),
        };
        let ready = permit.is_some();
        exchange.send_message(format, &[u8::from(ready)]).await?;
        let peer_ready = match exchange.recv_message(format).await?.as_slice() {
            [1] => true,
            [0] => false,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        if peer_ready && let Some(physical) = permit { break io::Result::Ok(physical); }
        // Neither peer has sampled. Release the ready side while the busy
        // side waits for its existing owner, then rendezvous before retrying.
        drop(permit);
        if !ready { changed.changed().await.map_err(|_| io::ErrorKind::NotConnected)?; }
        exchange.send_message(format, &[1]).await?;
        if exchange.recv_message(format).await? != [1] { return Err(io::ErrorKind::InvalidData.into()); }
    }};
    let physical = tokio::select! { biased;
        _ = async {
            if let Some((_, validity)) = &binding {
                control.endpoint.inner.wait_for_binding_loss(validity).await;
            } else {
                std::future::pending().await
            }
        } => return Err(io::ErrorKind::Interrupted.into()),
        result = rendezvous => result?,
    };
    let prepared = direct.prepare_bootstrap(control, &exchange, physical).await?;
    let remote = direct::verify_direct(control, prepared)
        .await?
        .commit(control, &exchange)
        .await?;
    let path = control
        .paths()
        .iter()
        .find(|path| path.remote_addr() == &remote)
        .map(|path| path.id())
        .ok_or(io::ErrorKind::NotConnected)?;
    control.qualify_data_path(path, custom).await
}

async fn udp(
    control: &ControlHandle,
    exchange: ControlExchange,
    custom: Vec<u64>,
    family: ObservationFamily,
    ip_data_enabled: bool,
) -> io::Result<DataPathPermit> {
    let format = ControlMessageFormat::Fixed { length: 1 };
    if !ip_data_enabled {
        exchange.send_message(format, &[0]).await?;
        exchange.finish_send().await?;
        return Err(io::ErrorKind::Unsupported.into());
    }
    let policy = control
        .endpoint
        .inner
        .static_config
        .admission
        .as_ref()
        .expect("member policy");
    let paths = control.paths_stream();
    let qualify = async {
        tokio::pin!(paths);
        while let Some(paths) = paths.next().await {
            if let Some(path) = paths
                .iter()
                .find(|path| matches!(path.remote_addr(), super::TransportAddr::Ip(addr) if family.matches(*addr)) && policy.permits_member_data_path(path.remote_addr()))
            {
                return control.qualify_data_path(path.id(), custom.clone()).await;
            }
        }
        Err(io::ErrorKind::NotConnected.into())
    };
    tokio::pin!(qualify);
    let mut direct = None;
    let enhanced = if let Some(udp) = control.member_udp.as_deref() {
        tokio::select! { biased;
            result = &mut qualify => { direct = Some(result?); None },
            needed = udp.bootstrap_requested(family) => needed.then_some(udp),
        }
    } else {
        None
    };
    exchange
        .send_message(format, &[u8::from(enhanced.is_some())])
        .await?;
    match exchange.recv_message(format).await?.as_slice() {
        [1] if let Some(udp) = enhanced => {
            let deadline = control
                .caller_deadline()
                .expect("restricted admission deadline")
                .into();
            let cancel = control.attempt_cancellation().unwrap_or_default();
            match udp.bootstrap(family, exchange, deadline, &cancel).await {
                Ok(path) => return control.qualify_data_path(path, custom.clone()).await,
                Err(error) => tracing::debug!(stable_id = control.stable_id(), %error,
                    "enhanced UDP did not qualify the restricted member contact"),
            }
        }
        [0] | [1] => exchange.finish_send().await?,
        _ => return Err(io::ErrorKind::InvalidData.into()),
    }
    if let Some(permit) = direct {
        Ok(permit)
    } else {
        qualify.await
    }
}
