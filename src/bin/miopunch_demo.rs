//! Invited-member Linux demo host. The shared SDK and its defaults are unchanged.
#[path = "../../demo/host/services.rs"]
mod services;

use iroh::{
    endpoint::{Connection, ControlConnection, presets},
    protocol::{AcceptError, ControlProtocolHandler, ProtocolHandler, RestrictedRouter},
};
use iroh_relay::tls::CaTlsConfig;
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, PRECODE_REQUEST_ALPN, ProtocolScope},
    connection_coordinator::{ACTIVE_IMPROVEMENT_ALPN, MEMBER_ALPN},
    control_protocol::{AuthenticatedControl, OwnedControl},
    native_network::NativeNetwork,
    resources::{Resource, ResourceUse},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};

fn connection_paths(connection: &iroh::endpoint::Connection) -> Vec<serde_json::Value> {
    connection
        .paths()
        .iter()
        .map(|path| {
            let stats = path.stats();
            json!({
                "route": path.remote_addr().to_string(),
                "selected": path.is_selected(),
                "isRelay": path.is_relay(),
                "isIp": path.is_ip(),
                "isRtc": matches!(path.remote_addr(), iroh::TransportAddr::Custom(address)
                    if address.id() == miopunch_native_probe::rtc_transport::TRANSPORT_ID),
                "sentBytes": stats.udp_tx.bytes,
                "receivedBytes": stats.udp_rx.bytes,
                "streamSent": stats.frame_tx.stream,
                "streamReceived": stats.frame_rx.stream,
                "rttMicros": stats.rtt.as_micros(),
            })
        })
        .collect()
}

#[derive(Clone)]
struct DemoMember {
    gate: AdmissionGate,
    services: Arc<tokio::sync::Mutex<services::Services>>,
    slots: Arc<tokio::sync::Semaphore>,
    rtc: miopunch_native_probe::rtc_transport::RtcTransport,
    host_id: iroh::EndpointId,
}
impl std::fmt::Debug for DemoMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DemoMember")
    }
}
impl DemoMember {
    async fn serve(&self, connection: Connection) -> Result<(), services::Failure> {
        let _slot = self.slots.clone().try_acquire_owned()?;
        let peer = connection.remote_id().to_string();
        loop {
            let streams =
                tokio::time::timeout(Duration::from_secs(180), connection.accept_bi()).await;
            let Ok(Ok((mut send, mut recv))) = streams else {
                break;
            };
            if !self.gate.allows(&connection, ProtocolScope::Member) {
                connection.close(0u32.into(), b"membership refused");
                break;
            }
            let bytes =
                tokio::time::timeout(Duration::from_secs(20), recv.read_to_end(60000)).await??;
            let response = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(request) if request["op"] == "connection.info" => Ok(json!({
                    "hostId": self.host_id.to_string(), "remoteId": peer,
                    "stableId": connection.stable_id(), "paths": connection_paths(&connection),
                    "rtcAssociations": self.rtc.active_associations(),
                    "rtcPumps": self.rtc.active_pumps(),
                })),
                Ok(request) => self.services.lock().await.request(&peer, request).await,
                Err(error) => Err(error.into()),
            }
            .unwrap_or_else(|error| json!({"error": error.to_string()}));
            let response = serde_json::to_vec(&response)?;
            if response.len() > 64000 {
                return Err("demo response limit exceeded".into());
            }
            send.write_all(&response).await?;
            send.finish()?;
        }
        Ok(())
    }
}
impl ProtocolHandler for DemoMember {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.serve(connection)
            .await
            .map_err(AcceptError::from_boxed)
    }
}

#[derive(Clone)]
struct DemoPrecode {
    node: Arc<tokio::sync::Mutex<NativeNetwork>>,
    slots: Arc<tokio::sync::Semaphore>,
}
impl std::fmt::Debug for DemoPrecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DemoPrecode")
    }
}
impl ControlProtocolHandler for DemoPrecode {
    async fn accept(&self, connection: ControlConnection) -> Result<(), AcceptError> {
        let result = async {
            let _slot = self.slots.clone().try_acquire_owned()?;
            let control = Arc::new(connection).retain_control();
            self.node
                .lock()
                .await
                .serve_precode_admission(&control)
                .await?;
            control.revocation_handle().closed().await;
            Ok::<_, services::Failure>(())
        }
        .await;
        result.map_err(AcceptError::from_boxed)
    }
}

#[tokio::main]
async fn main() -> Result<(), services::Failure> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err(
            "usage: miopunch_demo PRIVATE_STATE RELAY_URL RELAY_CERT_DER TERMINAL_HELPER".into(),
        );
    }
    let directory = PathBuf::from(&args[1]);
    let node_path = directory.join("node");
    let mut node = if node_path.exists() {
        NativeNetwork::open(&node_path).await?
    } else {
        NativeNetwork::create(&node_path).await?
    };
    let ice: miopunch_native_probe::rtc_ice_profile::RtcIceProfile =
        serde_json::from_str(&std::env::var("DEMO_RTC_ICE").unwrap_or_else(|_| "{}".into()))?;
    ice.validate()?;
    let mut resources = node.resource_configuration()?;
    let mut allowed = vec![Resource {
        url: args[2].clone(),
        uses: [ResourceUse::DataRelay].into(),
        limitation: "explicit demo relay".into(),
    }];
    allowed.extend(
        ice.observation_resources()?
            .into_iter()
            .map(|url| Resource {
                url,
                uses: [ResourceUse::Observation].into(),
                limitation: "explicit demo RTC STUN observation; no TURN".into(),
            }),
    );
    resources.import_trusted_replacement(allowed)?;
    resources.udp_direct_disabled = false;
    resources.tcp_direct_disabled = false;
    resources.path_selection.preference =
        miopunch_native_probe::path_selection::PathPreference::PreferDirect;
    resources.lan_discovery = miopunch_native_probe::resources::LanDiscovery::Disabled;
    node.save_resource_configuration(&resources)?;
    node.admission_gate()
        .register_protocol(MEMBER_ALPN, ProtocolScope::Member);
    let tls = if args[3] == "-" {
        CaTlsConfig::embedded()
    } else {
        CaTlsConfig::embedded().with_extra_roots([rustls::pki_types::CertificateDer::from(
            std::fs::read(&args[3])?,
        )])
    };
    let rtc = miopunch_native_probe::rtc_transport::RtcTransport::with_ice_profile(ice)?;
    let resources = Arc::new(
        node.bind_configured_endpoint_with_rtc(
            iroh::Endpoint::builder(presets::Minimal).ca_tls_config(tls),
            rtc.clone(),
        )
        .await?,
    );
    let endpoint = resources.endpoint().clone();
    tokio::time::timeout(Duration::from_secs(30), endpoint.online()).await?;
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        + 7 * 24 * 3600;
    let code = node.issue_precode(expires, Some(32)).await?;
    // The supervisor consumes this private pipe; it never writes this to logs.
    println!(
        "{}",
        json!({"event":"ready","peer":endpoint.id().to_string(),"code":hex::encode(code.encode())})
    );
    let active_handler = node.active_improvement_handler(resources.clone());
    let gate = node.admission_gate().clone();
    let node = Arc::new(tokio::sync::Mutex::new(node));
    let services = Arc::new(tokio::sync::Mutex::new(services::Services::new(
        directory.join("services"),
        PathBuf::from(&args[4]),
    )));
    let slots = Arc::new(tokio::sync::Semaphore::new(24));
    let router = RestrictedRouter::builder(endpoint.clone())
        .accept(
            MEMBER_ALPN,
            DemoMember {
                gate,
                services,
                slots: slots.clone(),
                rtc,
                host_id: endpoint.id(),
            },
        )
        .accept_control(
            PRECODE_REQUEST_ALPN,
            DemoPrecode {
                node: node.clone(),
                slots,
            },
        )
        .accept_control(ACTIVE_IMPROVEMENT_ALPN, active_handler)
        .spawn();
    tokio::signal::ctrl_c().await?;
    router.shutdown().await?;
    drop(router);
    Arc::try_unwrap(resources)
        .map_err(|_| "demo resource handler still retained")?
        .close()
        .await;
    Arc::try_unwrap(node)
        .map_err(|_| "node still retained")?
        .into_inner()
        .close()
        .await;
    Ok(())
}
