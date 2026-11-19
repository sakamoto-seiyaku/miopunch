use std::{io::ErrorKind, path::Path, process, time::Duration};

use iroh::{
    EndpointAddr, EndpointId, RelayUrl, TransportAddr,
    endpoint::{ConnectingError, Connection, RestrictedEndpoint},
};
use miopunch_native_probe::{
    admission_gate::ProtocolScope,
    connection_coordinator::connection_diagnostic::monotonic_ms,
    daily_management::ManagementAction,
    native_network::{FUSED_MEMBER_ALPN, NativeNetwork, member_fusion_attempt_id},
    resources::{ResourceConfig, ResourceTable},
};
use serde_json::{Value, json};
use tokio::process::Command;

use super::{admit, bind, relay, resource};

const SETUP: Duration = Duration::from_secs(5);
const REQUEST: &[u8] = b"fusion ping 143";
const REPLY: &[u8] = b"fusion pong 143";

#[tokio::test]
async fn approved_member_gets_first_relay_reply_and_revoked_member_cannot_open_a_stream() {
    let service = relay().await;
    let url = service.http_url().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let join_directory = dir.path().join("joining");
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let mut joining = NativeNetwork::join(&join_directory, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let mut config = ResourceConfig::with_defaults(ResourceTable {
        revision: 1,
        resources: vec![resource(&url)],
    });
    config.udp_direct_disabled = true;
    config.tcp_direct_disabled = true;
    for node in [&admin, &joining] {
        node.save_resource_configuration(&config).unwrap();
    }
    let admin_bootstrap = bind(&admin).await;
    let join_bootstrap = bind(&joining).await;
    admit(
        &mut admin,
        &mut joining,
        &admin_bootstrap,
        &join_bootstrap,
        &url,
    )
    .await;
    let member_id = joining.status().node_id;
    tokio::join!(admin_bootstrap.close(), join_bootstrap.close());
    joining.close().await;

    let server = admin.bind_member_fusion_endpoint(SETUP).await.unwrap();
    tokio::time::timeout(SETUP, server.online()).await.unwrap();
    let started = monotonic_ms();
    let (dialer, responder) = tokio::time::timeout(SETUP * 2, async {
        tokio::join!(dial(&join_directory, &server, &url, "approved"), async {
            let connection = server.accept().await.unwrap().await.unwrap();
            assert_eq!(connection.remote_id().as_bytes(), member_id.as_bytes());
            assert!(
                admin
                    .admission_gate()
                    .allows(&connection, ProtocolScope::Member)
            );
            let stable_id = connection.stable_id();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            assert_eq!(recv.read_to_end(64).await.unwrap(), REQUEST);
            send.write_all(REPLY).await.unwrap();
            send.finish().unwrap();
            assert_eq!(send.stopped().await.unwrap(), None);
            assert_eq!(connection.stable_id(), stable_id);
            let report = reply_record(&connection, &url, "responder", started);
            eprintln!("{report}");
            report
        },)
    })
    .await
    .unwrap();
    let dialer = output_record(&dialer, "fusion-first-reply", "initiator");
    assert_eq!(dialer["attemptId"], responder["attemptId"]);
    assert_eq!(dialer["round"], responder["round"]);
    assert_ne!(dialer["pid"], responder["pid"]);
    assert_eq!(dialer["peer"], server.id().to_string());

    let mut manager = admin.begin_management_session();
    admin
        .manage(&mut manager, ManagementAction::RemoveMember(member_id))
        .await
        .unwrap();
    assert!(
        !admin
            .status()
            .members
            .iter()
            .any(|(id, _)| *id == member_id)
    );
    let (dialer, denied) = tokio::time::timeout(SETUP * 2, async {
        tokio::join!(dial(&join_directory, &server, &url, "revoked"), async {
            server.accept().await.unwrap().await
        },)
    })
    .await
    .unwrap();
    assert!(
        matches!(
            denied,
            Err(ConnectingError::AdmissionFailed {
                kind: ErrorKind::PermissionDenied,
                ..
            })
        ),
        "responder must reach admission and deny business: {denied:?}"
    );
    let rejection = output_record(&dialer, "connection-timeline", "initiator");
    assert_ne!(rejection["attemptId"], responder["attemptId"]);
    assert_eq!(rejection["round"], 1);
    server.close().await;
    admin.close().await;
    service.shutdown().await.unwrap();
}

async fn dial(
    directory: &Path,
    server: &RestrictedEndpoint,
    relay: &RelayUrl,
    expected: &str,
) -> process::Output {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "member_fusion::dialer_process",
            "--nocapture",
        ])
        .env("MIOPUNCH_FUSION_DIRECTORY", directory)
        .env("MIOPUNCH_FUSION_PEER", server.id().to_string())
        .env("MIOPUNCH_FUSION_RELAY", relay.as_str())
        .env("MIOPUNCH_FUSION_EXPECTED", expected)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

#[tokio::test]
#[ignore = "launched as a separate native process by the relay acceptance test"]
async fn dialer_process() {
    let directory = std::env::var_os("MIOPUNCH_FUSION_DIRECTORY").unwrap();
    let peer: EndpointId = std::env::var("MIOPUNCH_FUSION_PEER")
        .unwrap()
        .parse()
        .unwrap();
    let relay: RelayUrl = std::env::var("MIOPUNCH_FUSION_RELAY")
        .unwrap()
        .parse()
        .unwrap();
    let expected = std::env::var("MIOPUNCH_FUSION_EXPECTED").unwrap();
    let node = NativeNetwork::open(directory).await.unwrap();
    assert_eq!(
        node.admission_stage(),
        miopunch_native_probe::manual_admission::AdmissionStage::Complete
    );
    let endpoint = node.bind_member_fusion_endpoint(SETUP).await.unwrap();
    let started = monotonic_ms();
    let result = endpoint
        .connect(
            EndpointAddr::new(peer).with_relay_url(relay.clone()),
            FUSED_MEMBER_ALPN,
        )
        .await;
    if expected == "approved" {
        let connection = result.unwrap();
        assert_eq!(connection.remote_id(), peer);
        assert!(
            node.admission_gate()
                .allows(&connection, ProtocolScope::Member)
        );
        let stable_id = connection.stable_id();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(REQUEST).await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(64).await.unwrap(), REPLY);
        assert_eq!(connection.stable_id(), stable_id);
        eprintln!(
            "{}",
            reply_record(&connection, &relay, "initiator", started)
        );
    } else {
        assert_eq!(expected, "revoked");
        assert!(result.is_err(), "revoked member received a business handle");
    }
    endpoint.close().await;
    node.close().await;
}

fn reply_record(connection: &Connection, relay: &RelayUrl, role: &str, started: u64) -> Value {
    let relay_path = TransportAddr::Relay(relay.clone());
    assert!(
        connection
            .paths()
            .iter()
            .any(|path| path.is_selected() && path.remote_addr() == &relay_path)
    );
    let finished = monotonic_ms();
    json!({
        "event": "fusion-first-reply",
        "attemptId": hex::encode(member_fusion_attempt_id(connection).unwrap()),
        "round": 1,
        "pid": process::id(),
        "stableId": connection.stable_id(),
        "monotonicMs": finished,
        "durationMs": finished - started,
        "role": role,
        "peer": connection.remote_id().to_string(),
        "relay": relay.as_str(),
    })
}

fn output_record(output: &process::Output, event: &str, role: &str) -> Value {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| record["event"] == event && record["role"] == role)
        .unwrap()
}
