//! Private relay candidate preparation for the common coordinator.
use crate::admission_gate::AdmissionGate;
use iroh::TransportAddr;
use std::{io, time::Duration};

fn debug_relay_check(
    connection: &iroh::endpoint::ControlHandle,
    gate: &AdmissionGate,
    resources: &crate::resources::ResourceConfig,
    phase: &str,
    mut details: serde_json::Value,
) {
    use n0_watcher::Watcher;
    let home = connection.home_relay_status().get();
    let paths = connection.paths();
    let selected = paths.iter().find(|path| path.is_selected());
    let selected_kind = match selected.as_ref().map(|path| path.remote_addr()) {
        Some(TransportAddr::Relay(_)) => "relay",
        Some(TransportAddr::Custom(address))
            if matches!(
                address.id(),
                crate::mqtt_datagram::TRANSPORT_ID | crate::irc_datagram::TRANSPORT_ID
            ) =>
        {
            "signal"
        }
        Some(TransportAddr::Ip(_)) => "ip",
        Some(_) => "other",
        None => "none",
    };
    details["tag"] = serde_json::json!("DEBUG-mio76-relay");
    details["phase"] = serde_json::json!(phase);
    details["sideClient"] = serde_json::json!(connection.side().is_client());
    details["memberOpen"] = serde_json::json!(connection.close_reason().is_none());
    details["gateValid"] = serde_json::json!(gate.allows_member_path_upgrade(connection));
    details["configuredPermittedCount"] = serde_json::json!(
        resources
            .relay_map(crate::resources::ResourceUse::DataRelay)
            .ok()
            .map(|map| map.urls::<Vec<_>>().len())
    );
    details["localPublishedCount"] = serde_json::json!(connection.addr().relay_urls().count());
    details["localConnectedCount"] =
        serde_json::json!(home.iter().filter(|status| status.is_connected()).count());
    details["localRelayErrorCount"] = serde_json::json!(
        home.iter()
            .filter(|status| status.last_error().is_some())
            .count()
    );
    details["relayPathCount"] =
        serde_json::json!(paths.iter().filter(|path| path.is_relay()).count());
    details["permittedRelayPathCount"] = serde_json::json!(paths.iter().filter(|path| matches!(path.remote_addr(), TransportAddr::Relay(url) if resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))).count());
    details["selectedRouteKind"] = serde_json::json!(selected_kind);
    details
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    eprintln!("[DEBUG-mio76-relay] {details}");
}

fn relay_probe_debug_details(
    connection: &iroh::endpoint::ControlHandle,
    exact_path: Option<&str>,
    elapsed_ms: u128,
    stage_remaining_ms: Option<u128>,
    started_utc_ms: Option<u128>,
) -> serde_json::Value {
    let mut details = serde_json::json!({"elapsedMs":elapsed_ms});
    details["stageRemainingMs"] = serde_json::json!(stage_remaining_ms);
    details["startedUtcMs"] = serde_json::json!(started_utc_ms);
    details["callerRemainingMs"] =
        serde_json::json!(connection.caller_deadline().map(|deadline| {
            deadline
                .saturating_duration_since(n0_future::time::Instant::now())
                .as_millis()
        }));
    if let Some(exact_path) = exact_path {
        let paths = connection.paths();
        let path = paths
            .iter()
            .find(|path| path.id().to_string() == exact_path);
        details["exactProbePathPresent"] = serde_json::json!(path.is_some());
        if let Some(path) = path {
            let stats = path.stats();
            details["probePathSelected"] = serde_json::json!(path.is_selected());
            details["probePathRttMicros"] = serde_json::json!(stats.rtt.as_micros());
            details["probePingTx"] = serde_json::json!(stats.frame_tx.ping);
            details["probePingRx"] = serde_json::json!(stats.frame_rx.ping);
            details["probeAckTx"] = serde_json::json!(stats.frame_tx.acks);
            details["probeAckRx"] = serde_json::json!(stats.frame_rx.acks);
            details["probePathAckTx"] = serde_json::json!(stats.frame_tx.path_acks);
            details["probePathAckRx"] = serde_json::json!(stats.frame_rx.path_acks);
            details["probeDatagramsTx"] = serde_json::json!(stats.udp_tx.datagrams);
            details["probeDatagramsRx"] = serde_json::json!(stats.udp_rx.datagrams);
            details["probeBytesTx"] = serde_json::json!(stats.udp_tx.bytes);
            details["probeBytesRx"] = serde_json::json!(stats.udp_rx.bytes);
            details["probeLostPackets"] = serde_json::json!(stats.lost_packets);
        }
    }
    details
        .as_object_mut()
        .unwrap()
        .retain(|_, value| !value.is_null());
    details
}

pub(crate) async fn prepare_relay_control(
    connection: &iroh::endpoint::ControlHandle,
    gate: &AdmissionGate,
    resources: &crate::resources::ResourceConfig,
) -> io::Result<()> {
    let check = || {
        if !gate.allows_member_path_upgrade(connection) || connection.close_reason().is_some() {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "relay member scope changed",
            ))
        } else {
            Ok(())
        }
    };
    debug_relay_check(connection, gate, resources, "entry", serde_json::json!({}));
    check()?;
    let local: Vec<_> = connection
        .addr()
        .relay_urls()
        .filter(|url| resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))
        .take(4)
        .cloned()
        .collect();
    let local_snapshot_count = local.len();
    debug_relay_check(
        connection,
        gate,
        resources,
        "begin",
        serde_json::json!({"localCandidateCount":local_snapshot_count}),
    );
    let exchange = crate::member_path_protocol::RelayValidation::begin(connection, local).await?;
    debug_relay_check(
        connection,
        gate,
        resources,
        "exchange",
        serde_json::json!({"localCandidateCount":local_snapshot_count,"remoteCandidateCount":exchange.candidates.len(),"mutuallyAllowedCount":exchange.candidates.iter().filter(|url| resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay)).count()}),
    );
    let stream_id = exchange.stream_id.clone();
    let remote = exchange
        .candidates
        .iter()
        .find(|url| resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))
        .cloned();
    let validation = async {
            let remote = remote.ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "no mutually permitted relay"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
            let started = tokio::time::Instant::now();
            let started_utc_ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|value|value.as_millis());
            let mut fresh = None;
            let samples=[1u64,2,4,6];
            let mut sampled=0;
            loop {
                check()?;
                if connection.side().is_client() {
                    match connection.open_control_relay_path(remote.clone()).await {
                        Ok(()) => {},
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {},
                        Err(error) => return Err(error),
                    }
                }
                if let Some(path) = connection.paths().iter().find(|path| matches!(path.remote_addr(), TransportAddr::Relay(url)
                    if resources.allows(url.as_str(), crate::resources::ResourceUse::DataRelay))) {
                    if fresh.is_none() {
                        let probe = path.start_probe()?;
                        let mut details=relay_probe_debug_details(connection,Some(&path.id().to_string()),started.elapsed().as_millis(),Some(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()),started_utc_ms);
                        details["freshProbeIssued"]=serde_json::json!(true);
                        details["probeWatermark"]=serde_json::json!(probe.watermark());
                        debug_relay_check(connection, gate, resources, "probe-start", details);
                        fresh = Some((path.id().to_string(), probe, started.elapsed().as_millis()));
                    }
                    let (path_id, probe, probe_started) = fresh.as_ref().unwrap();
                    if path.id().to_string() != *path_id { return Err(io::Error::other("relay probe path changed")); }
                    if probe.is_acknowledged().map_err(io::Error::other)? {
                        let mut details=relay_probe_debug_details(connection,Some(path_id),started.elapsed().as_millis(),Some(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()),started_utc_ms);
                        details["freshAck"]=serde_json::json!(true);
                        details["probeWatermark"]=serde_json::json!(probe.watermark());
                        debug_relay_check(connection, gate, resources, "probe-ack", details);
                        return Ok(serde_json::json!({"pathId":path_id,"freshProbeStartedMs":probe_started,"freshAckMs":started.elapsed().as_millis(),"probeWatermark":probe.watermark(),"freshAck":true}));
                    }
                }
                if samples.get(sampled).is_some_and(|seconds| started.elapsed()>=Duration::from_secs(*seconds)) {
                    let mut details=relay_probe_debug_details(connection,fresh.as_ref().map(|(path_id,_,_)|path_id.as_str()),started.elapsed().as_millis(),Some(deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis()),started_utc_ms);
                    if let Some((_,probe,_))=fresh.as_ref() { details["probeWatermark"]=serde_json::json!(probe.watermark()); details["freshAck"]=serde_json::json!(probe.is_acknowledged().ok()); }
                    debug_relay_check(connection, gate, resources, "probe-progress", details);
                    sampled+=1;
                }
                if tokio::time::Instant::now() >= deadline { return Err(io::Error::new(io::ErrorKind::TimedOut, "relay data validation timed out")); }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }.await;
    debug_relay_check(
        connection,
        gate,
        resources,
        "validation-end",
        serde_json::json!({"localCandidateCount":local_snapshot_count,"validationOk":validation.is_ok(),"errorKind":validation.as_ref().err().map(|error|format!("{:?}",error.kind()))}),
    );
    if let Err(error) = &validation {
        eprintln!(
            "MESSAGE_RELAY_VALIDATION_FAILED stable_id={} error={error}",
            connection.stable_id()
        );
    }
    let observation = validation.as_ref().ok().cloned();
    let finished = exchange.finish(connection, validation.map(|_| ())).await;
    debug_relay_check(
        connection,
        gate,
        resources,
        "finish",
        serde_json::json!({"finishOk":finished.is_ok(),"errorKind":finished.as_ref().err().map(|error|format!("{:?}",error.kind()))}),
    );
    let continuity = finished?;
    check()?;
    println!(
        "{}",
        serde_json::json!({"event":"relay-path", "stableId":connection.stable_id(),"privateStreamId":stream_id,
            "stableUnchanged":continuity.stable_unchanged,"bindingUnchanged":continuity.binding_unchanged,"probe":observation,
            "relayFramesAfterConfirmation":connection.paths().iter().filter(|path|path.is_relay()).map(|path|serde_json::json!({"pathId":path.id().to_string(),"selected":path.is_selected(),"streamSent":path.stats().frame_tx.stream,"streamReceived":path.stats().frame_rx.stream})).collect::<Vec<_>>()})
    );
    println!(
        "MESSAGE_RELAY_VALIDATED stable_id={} peer={} paths={:?}",
        connection.stable_id(),
        connection.remote_id(),
        connection.paths()
    );
    Ok(())
}
