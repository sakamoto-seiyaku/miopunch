//! Concurrent paths on the protected first-contact connection.

use std::io;

use futures_util::StreamExt;

use super::{
    ControlConnection, ControlExchange, ControlHandle, ControlMessageFormat, DataPathPermit,
    MEMBER_DATA_UNAVAILABLE, direct, time,
};

/// Only these already allocated internal exchanges outlive application-control
/// retirement. The worker remains bound to the original attempt and connection.
pub(super) async fn qualify(
    connection: &ControlConnection,
    custom: Vec<u64>,
) -> io::Result<DataPathPermit> {
    let control = connection.member_bootstrap_handle();
    control.prepare_member_udp().await?;
    let tcp_exchange = exchange(&control, 1).await?;
    let udp_exchange = exchange(&control, 2).await?;
    let deadline = control
        .caller_deadline()
        .expect("restricted admission deadline");
    let cancel = control.attempt_cancellation().unwrap_or_default();
    let generation = control.udp_network_generation();
    let (completed, mut results) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let tcp = async {
            let result = tcp(&control, tcp_exchange, custom.clone()).await;
            let _ = completed.send(result);
        };
        let udp = async {
            let result = udp(&control, udp_exchange, custom.clone()).await;
            let _ = completed.send(result);
        };
        tokio::select! { biased;
            _ = control.closed() => {},
            _ = cancel.cancelled() => {},
            _ = control.endpoint.inner.wait_network_generation_change(&generation) => {},
            _ = time::sleep_until(deadline.into()) => {},
            _ = async { tokio::join!(tcp, udp); } => {},
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
) -> io::Result<DataPathPermit> {
    let policy = control
        .endpoint
        .inner
        .static_config
        .admission
        .as_ref()
        .expect("member policy");
    let supported = policy.member_direct_traversal().is_some()
        && policy
            .member_direct_transport()
            .is_some_and(|id| custom.contains(&id));
    let format = ControlMessageFormat::Fixed { length: 1 };
    exchange.send_message(format, &[u8::from(supported)]).await?;
    match exchange.recv_message(format).await?.as_slice() {
        [1] if supported => {},
        [0] | [1] => return Err(io::ErrorKind::Unsupported.into()),
        _ => return Err(io::ErrorKind::InvalidData.into()),
    }
    let deadline = control.caller_deadline().expect("restricted admission deadline").into();
    let building = &policy.member_fusion().expect("member fusion").building;
    let observers = policy.member_direct_observers();
    let binding = if let Some(configured) = policy.member_direct_binding().filter(|_| !observers.is_empty()) {
        Some(super::physical::observer_binding(
            &control.endpoint, iroh_relay::ObservationFamily::Ipv4,
            observers.into_iter().map(|url| (url, 3478)), Some(configured), deadline,
        ).await?)
    } else { None };
    let physical = loop {
        let mut changed = building.subscribe();
        let permit = match binding {
            Some(binding) => {
                let scope = control.endpoint.inner.punching_scope(binding, true)?;
                building.physical.try_acquire(scope, control.remote_id()).map(|permit| permit.with_binding(binding))
            },
            None => Some(super::physical::MemberPunching::untracked()),
        };
        let ready = permit.is_some();
        exchange.send_message(format, &[u8::from(ready)]).await?;
        let peer_ready = match exchange.recv_message(format).await?.as_slice() {
            [1] => true,
            [0] => false,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        if ready && peer_ready { break permit; }
        // Neither peer has sampled. Release the ready side while the busy
        // side waits for its existing owner, then rendezvous before retrying.
        drop(permit);
        if !ready { changed.changed().await.map_err(|_| io::ErrorKind::NotConnected)?; }
        exchange.send_message(format, &[1]).await?;
        if exchange.recv_message(format).await? != [1] { return Err(io::ErrorKind::InvalidData.into()); }
    };
    let prepared = match physical {
        Some(physical) => {
            policy
                .prepare_member_bootstrap_direct_reserved(control, &exchange, physical)
                .await?
        }
        None => {
            policy
                .prepare_member_bootstrap_direct(control, &exchange)
                .await?
        }
    };
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
) -> io::Result<DataPathPermit> {
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
                .find(|path| path.is_ip() && policy.permits_member_data_path(path.remote_addr()))
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
            needed = udp.bootstrap_requested() => needed.then_some(udp),
        }
    } else {
        None
    };
    let format = ControlMessageFormat::Fixed { length: 1 };
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
            match udp.bootstrap(exchange, deadline, &cancel).await {
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
