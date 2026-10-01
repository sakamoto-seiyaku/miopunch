//! Public native-node behavior with real signed records, SQLite and iroh TLS.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr};
use miopunch_native_probe::admission_gate::{ADMISSION_REQUEST_ALPN, ProtocolScope};
use miopunch_native_probe::management_auth::ManagementExtension;
use miopunch_native_probe::manual_admission::{
    AdmissionStage, ManualAdmissionError, ManualExchange,
};
use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
use miopunch_native_probe::native_network::NativeNetwork;
use p2panda_auth::Access;
use p2panda_auth::GroupsExtensionArgs;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Header, SigningKey};

const BUSINESS: &[u8] = b"miopunch/manual-admission-business/1";

async fn connect(client: &Endpoint, server: &Endpoint, alpn: &[u8]) -> (Connection, Connection) {
    let target = EndpointAddr::new(server.id()).with_addrs(
        server
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        let (out, incoming) = tokio::join!(client.connect(target, alpn), async {
            server.accept().await.unwrap().await
        });
        (out.unwrap(), incoming.unwrap())
    })
    .await
    .expect("bounded real TLS connection")
}

#[tokio::test]
async fn manual_approval_saves_both_materials_and_permits_real_business() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let join_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let mut joining = NativeNetwork::join(&join_path, invitation).await.unwrap();
    let join_id = joining.status().node_id;
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    let admin_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let join_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    admin_endpoint.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);

    let (client, server) = connect(&join_endpoint, &admin_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    assert_eq!(request.applicant, join_id);
    let approved = admin.approve_manual_admission(&request).await.unwrap();
    assert_eq!(approved.stage, AdmissionStage::Approved);

    // Start delivery on the same authenticated request. The receiver exposes
    // the authorization-save boundary without claiming completed admission.
    let delivery = async {
        let ManualExchange::Materials(materials) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved request must deliver authorization");
        };
        assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
        joining.complete_manual_admission(materials).await.unwrap();
        assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    };
    let (_, delivered) = tokio::join!(delivery, admin.serve_manual_admission(&server));
    assert_eq!(delivered.unwrap(), AdmissionStage::Complete);
    assert!(admin.status().members.contains(&(join_id, Access::read())));
    assert!(
        joining
            .status()
            .members
            .contains(&(join_id, Access::read()))
    );
    assert_eq!(admin.status().applied_count, 2);

    assert!(
        matches!(
            joining.manual_invitation(),
            Err(ManualAdmissionError::Rejected)
        ),
        "ordinary membership must not confer daily administration"
    );

    client.close(0u32.into(), b"admission complete");
    server.close(0u32.into(), b"admission complete");
    join_endpoint.close().await;
    joining.close().await;
    let joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.status().node_id, join_id);
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    joining
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    admin
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let join_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&join_endpoint, &admin_endpoint, BUSINESS).await;
    assert!(
        joining
            .admission_gate()
            .allows(&client, ProtocolScope::Member)
    );
    assert!(
        admin
            .admission_gate()
            .allows(&server, ProtocolScope::Member)
    );
    let (mut send, _) = client.open_bi().await.unwrap();
    send.write_all(b"authorized business after durable admission")
        .await
        .unwrap();
    send.finish().unwrap();
    let (_, mut recv) = server.accept_bi().await.unwrap();
    assert_eq!(
        recv.read_to_end(128).await.unwrap(),
        b"authorized business after durable admission"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    join_endpoint.close().await;
    admin_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}

// This test-binary worker deliberately exits without dropping the node/store.
// The parent drives the actual serving node and checks the reopened public
// view. It is a no-op during an ordinary test-suite invocation.
#[test]
fn applicant_crash_after_authorization_worker() {
    let Ok(path) = std::env::var("MIO30_CRASH_JOIN_PATH") else {
        return;
    };
    let contact: iroh_base::EndpointId = std::env::var("MIO30_CRASH_CONTACT")
        .unwrap()
        .parse()
        .unwrap();
    let addr = std::env::var("MIO30_CRASH_CONTACT_ADDR")
        .unwrap()
        .parse()
        .unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut node = NativeNetwork::open(path).await.unwrap();
        let endpoint = node.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
        let connection = endpoint
            .connect(
                EndpointAddr::new(contact).with_ip_addr(addr),
                ADMISSION_REQUEST_ALPN,
            )
            .await
            .unwrap();
        let ManualExchange::Materials(_delivery) =
            node.request_manual_admission(&connection).await.unwrap()
        else {
            panic!("worker must receive approved authorization");
        };
        assert_eq!(node.admission_stage(), AdmissionStage::PendingMaterials);
        eprintln!(
            "actual applicant process exits after durable authorization, before requesting secret"
        );
        std::process::exit(77);
    });
}

#[tokio::test]
async fn crash_after_only_authorization_never_succeeds_and_same_request_resumes_once() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let join_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::join(&join_path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let join_id = joining.status().node_id;
    let admin_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let join_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) = connect(&join_endpoint, &admin_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    client.close(0u32.into(), b"before approval disconnect");
    server.close(0u32.into(), b"before approval disconnect");
    join_endpoint.close().await;
    joining.close().await;
    let pending = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(pending.admission_stage(), AdmissionStage::AwaitingApproval);
    pending.close().await;
    let original = admin.approve_manual_admission(&request).await.unwrap();
    assert_eq!(original.stage, AdmissionStage::Approved);

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "applicant_crash_after_authorization_worker",
            "--nocapture",
        ])
        .env("MIO30_CRASH_JOIN_PATH", &join_path)
        .env("MIO30_CRASH_CONTACT", admin_endpoint.id().to_string())
        .env(
            "MIO30_CRASH_CONTACT_ADDR",
            admin_endpoint.addr().ip_addrs().next().unwrap().to_string(),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (child, delivered) = tokio::join!(
        tokio::task::spawn_blocking(move || child.wait_with_output().unwrap()),
        async {
            let server = admin_endpoint.accept().await.unwrap().await.unwrap();
            admin.serve_manual_admission(&server).await
        }
    );
    let child = child.unwrap();
    assert_eq!(
        child.status.code(),
        Some(77),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&child.stderr));
    assert!(matches!(
        delivered,
        Err(ManualAdmissionError::PathUnavailable)
    ));
    admin_endpoint.close().await;
    admin.close().await;

    let mut admin = NativeNetwork::open(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.status().node_id, join_id);
    assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    assert!(
        joining
            .status()
            .members
            .contains(&(join_id, Access::read()))
    );
    let retried = admin.approve_manual_admission(&request).await.unwrap();
    assert_eq!(retried.authorization, original.authorization);
    assert_eq!(retried.stage, AdmissionStage::Approved);
    assert_eq!(admin.status().applied_count, 2);
    assert_eq!(admin.manual_requests().await.unwrap(), vec![request]);

    admin
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    joining
        .admission_gate()
        .register_protocol(BUSINESS, ProtocolScope::Member);
    let admin_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let join_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    admin_endpoint.set_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec(), BUSINESS.to_vec()]);
    let target = EndpointAddr::new(admin_endpoint.id()).with_addrs(
        admin_endpoint
            .addr()
            .ip_addrs()
            .copied()
            .map(iroh_base::TransportAddr::Ip),
    );
    let (denied_client, denied_server) =
        tokio::join!(join_endpoint.connect(target, BUSINESS), async {
            admin_endpoint.accept().await.unwrap().await
        });
    assert!(
        denied_client.is_err() && denied_server.is_err(),
        "authorization alone must not admit normal business"
    );
    let (client, server) = connect(&join_endpoint, &admin_endpoint, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("must resume");
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (completed, delivered) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    assert_eq!(completed, AdmissionStage::Complete);
    assert_eq!(delivered.unwrap(), AdmissionStage::Complete);
    assert_eq!(
        admin.status().applied_count,
        2,
        "delivery retry cannot issue another Add"
    );
    assert_eq!(joining.status().applied_count, 2);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    join_endpoint.close().await;
    admin_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn invitation_for_wrong_network_cannot_queue_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let other_network = NativeNetwork::create(dir.path().join("other"))
        .await
        .unwrap();
    let mut invitation = admin.manual_invitation().unwrap();
    invitation.anchor = other_network.status().anchor;
    let mut joining = NativeNetwork::join(dir.path().join("joining"), invitation)
        .await
        .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (applicant, contact) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(contact, Err(ManualAdmissionError::Rejected)));
    assert!(applicant.is_err());
    assert!(admin.manual_requests().await.unwrap().is_empty());
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(joining.status().applied_count, 0);
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
    other_network.close().await;
}

#[tokio::test]
async fn actual_tls_contact_must_match_the_invitation_pin() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let other = NativeNetwork::create(dir.path().join("other"))
        .await
        .unwrap();
    let mut invitation = admin.manual_invitation().unwrap();
    invitation.contact = other.manual_invitation().unwrap().contact;
    let mut joining = NativeNetwork::join(dir.path().join("joining"), invitation)
        .await
        .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (applicant, contact) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(applicant, Err(ManualAdmissionError::Rejected)));
    assert!(contact.is_err());
    assert!(admin.manual_requests().await.unwrap().is_empty());
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
    other.close().await;
}

fn request_wire(
    invitation: miopunch_native_probe::manual_admission::ManualInvitation,
    request_id: [u8; 16],
    claimed: p2panda_core::VerifyingKey,
) -> Vec<u8> {
    let mut wire = invitation.encode().to_vec();
    wire.extend_from_slice(&request_id);
    wire.extend_from_slice(claimed.as_bytes());
    wire
}

#[tokio::test]
async fn forged_claimed_applicant_cannot_enter_the_approval_queue() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let holder = NativeNetwork::join(dir.path().join("holder"), invitation)
        .await
        .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let holder_endpoint = holder.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&holder_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let wire = request_wire(
        invitation,
        [19; 16],
        p2panda_core::SigningKey::generate().verifying_key(),
    );
    let forged = async {
        let (mut send, mut recv) = client.open_bi().await.unwrap();
        send.write_all(&wire).await.unwrap();
        send.finish().unwrap();
        recv.read_to_end(1).await
    };
    let (received, rejected) = tokio::join!(forged, admin.serve_manual_admission(&server));
    assert!(matches!(rejected, Err(ManualAdmissionError::Rejected)));
    assert!(received.is_err() || received.unwrap().is_empty());
    assert!(admin.manual_requests().await.unwrap().is_empty());
    assert_eq!(admin.status().applied_count, 1);
    holder_endpoint.close().await;
    server_endpoint.close().await;
    holder.close().await;
    admin.close().await;
}

async fn mqtt_endpoint(node: &NativeNetwork, broker: SocketAddr) -> (Endpoint, Arc<MqttTransport>) {
    let carrier = Arc::new(MqttTransport::new(broker, Arc::new(Traffic::default())));
    let endpoint = node
        .bind_endpoint_with(
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
                .control_only_alpns(vec![ADMISSION_REQUEST_ALPN.to_vec()])
                .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .unwrap()
                .clear_relay_transports()
                .add_custom_transport(carrier.clone()),
        )
        .await
        .unwrap();
    carrier.wait_ready().await.unwrap();
    (endpoint, carrier)
}

async fn mqtt_connect(
    client: &Endpoint,
    server: &Endpoint,
    carrier: &MqttTransport,
) -> (Connection, Connection) {
    let target = EndpointAddr::new(server.id()).with_addrs([iroh_base::TransportAddr::Custom(
        MqttTransport::address(&carrier.route().unwrap()).unwrap(),
    )]);
    tokio::time::timeout(Duration::from_secs(12), async {
        let (client, server) =
            tokio::join!(client.connect(target, ADMISSION_REQUEST_ALPN), async {
                server.accept().await.unwrap().await
            });
        (client.unwrap(), server.unwrap())
    })
    .await
    .unwrap()
}

struct MqttObserver {
    packets: Arc<Mutex<Vec<Vec<u8>>>>,
    worker: tokio::task::JoinHandle<()>,
}

impl MqttObserver {
    async fn start(broker: SocketAddr) -> Self {
        let options = rumqttc::MqttOptions::new(
            format!("mio30-holder-observer-{}", rand::random::<u64>()),
            broker.ip().to_string(),
            broker.port(),
        );
        let (client, mut events) = rumqttc::AsyncClient::new(options, 64);
        client
            .subscribe("miopunch/test/#", rumqttc::QoS::AtMostOnce)
            .await
            .unwrap();
        let packets = Arc::new(Mutex::new(Vec::new()));
        let captured = packets.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let mut ready = Some(ready);
            loop {
                match events.poll().await.unwrap() {
                    rumqttc::Event::Incoming(rumqttc::Packet::SubAck(_)) => {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    }
                    rumqttc::Event::Incoming(rumqttc::Packet::Publish(packet)) => {
                        captured.lock().unwrap().push(packet.payload.to_vec())
                    }
                    _ => {}
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(8), wait)
            .await
            .unwrap()
            .unwrap();
        Self { packets, worker }
    }
}

impl Drop for MqttObserver {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[tokio::test]
async fn another_invitation_holder_observes_ciphertext_but_cannot_take_an_approved_result() {
    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        eprintln!("SKIPPED actual invitation-holder MQTT check: MIOPUNCH_MQTT_BROKER absent");
        return;
    };
    let broker = broker.parse().unwrap();
    let observer = MqttObserver::start(broker).await;
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let secret = std::fs::read(admin_path.join("admission.secret")).unwrap();
    assert_eq!(invitation.encode().len(), 96);
    assert!(!invitation.encode().windows(32).any(|part| part == secret));
    let mut joining = NativeNetwork::join(dir.path().join("joining"), invitation)
        .await
        .unwrap();
    let holder = NativeNetwork::join(dir.path().join("holder"), invitation)
        .await
        .unwrap();
    assert_ne!(holder.status().node_id, joining.status().node_id);
    let (server_endpoint, server_carrier) = mqtt_endpoint(&admin, broker).await;
    let (client_endpoint, client_carrier) = mqtt_endpoint(&joining, broker).await;
    let (holder_endpoint, holder_carrier) = mqtt_endpoint(&holder, broker).await;
    let (client, server) = mqtt_connect(&client_endpoint, &server_endpoint, &server_carrier).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    let (intruder, contact) =
        mqtt_connect(&holder_endpoint, &server_endpoint, &server_carrier).await;
    let attempt = async {
        let (mut send, mut recv) = intruder.open_bi().await.unwrap();
        // Even disclosure of the original request id cannot merge identities.
        send.write_all(&request_wire(
            invitation,
            request.request_id,
            holder.status().node_id,
        ))
        .await
        .unwrap();
        send.finish().unwrap();
        recv.read_to_end(1).await.unwrap()
    };
    let (response, state) = tokio::join!(attempt, admin.serve_manual_admission(&contact));
    assert_eq!(
        response,
        [0],
        "a different holder cannot retrieve the approved history or secret"
    );
    assert_eq!(state.unwrap(), AdmissionStage::AwaitingApproval);
    assert_eq!(holder.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(admin.status().applied_count, 2);

    let receiver = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved materials");
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (complete, delivered) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    assert_eq!(complete, AdmissionStage::Complete);
    assert_eq!(delivered.unwrap(), AdmissionStage::Complete);
    assert!(
        !client
            .paths()
            .iter()
            .any(|path| matches!(path.remote_addr(), iroh_base::TransportAddr::Ip(_))),
        "this check must actually use the MQTT signaling carrier"
    );
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    intruder.close(0u32.into(), b"done");
    contact.close(0u32.into(), b"done");
    client_endpoint.close().await;
    holder_endpoint.close().await;
    server_endpoint.close().await;
    drop((client, server, intruder, contact));
    drop((client_endpoint, holder_endpoint, server_endpoint));
    tokio::time::timeout(Duration::from_secs(3), client_carrier.wait_closed())
        .await
        .expect("client carrier must be destroyed")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), holder_carrier.wait_closed())
        .await
        .expect("holder carrier must be destroyed")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), server_carrier.wait_closed())
        .await
        .expect("server carrier must be destroyed")
        .unwrap();
    let packets = observer.packets.lock().unwrap();
    assert!(!packets.is_empty());
    assert!(
        packets
            .iter()
            .all(|packet| !packet.windows(secret.len()).any(|part| part == secret))
    );
    assert!(
        packets
            .iter()
            .all(|packet| !packet.windows(96).any(|part| part == invitation.encode()))
    );
    eprintln!(
        "MIO30_MQTT_CIPHERTEXT publishes={} max_bytes={} long_lived_secret_in_invitation=false independent_holder_received_approved_result=false",
        packets.len(),
        packets.iter().map(Vec::len).max().unwrap()
    );
    drop(packets);
    joining.close().await;
    holder.close().await;
    admin.close().await;
}

#[tokio::test]
async fn original_result_is_rejected_after_removal_even_when_a_fresh_grant_readds_the_identity() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    let approved = admin.approve_manual_admission(&request).await.unwrap();
    assert_eq!(
        admin.approved_admission(&request).await.unwrap(),
        Some(approved.clone())
    );

    // An independently signed management fixture enters the real public
    // verifier; this is not a forged local membership flag or database edit.
    let key = SigningKey::from_bytes(
        &std::fs::read(admin_path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let remove = Header::<ManagementExtension>::builder()
        .build(
            &key,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: GroupsExtensionArgs {
                    group_id: admin.status().anchor.root,
                    action: GroupAction::Remove {
                        member: GroupMember::Individual(request.applicant),
                    },
                    dependencies: vec![approved.authorization],
                },
            },
        )
        .encode();
    let removal = admin.ingest_management_record(&remove, None).await.unwrap();
    assert!(
        !admin
            .status()
            .members
            .iter()
            .any(|(node, _)| *node == request.applicant)
    );
    assert!(matches!(
        admin.approved_admission(&request).await,
        Err(ManualAdmissionError::Rejected)
    ));
    assert!(matches!(
        admin.approve_manual_admission(&request).await,
        Err(ManualAdmissionError::Rejected)
    ));
    assert_eq!(admin.status().applied_count, 3);
    let readd = Header::<ManagementExtension>::builder()
        .build(
            &key,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: GroupsExtensionArgs {
                    group_id: admin.status().anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(request.applicant),
                        access: Access::read(),
                    },
                    dependencies: vec![removal],
                },
            },
        )
        .encode();
    admin.ingest_management_record(&readd, None).await.unwrap();
    assert!(
        admin
            .status()
            .members
            .contains(&(request.applicant, Access::read()))
    );
    assert!(
        matches!(
            admin.approved_admission(&request).await,
            Err(ManualAdmissionError::Rejected)
        ),
        "old result must not take over the fresh membership episode"
    );
    assert!(matches!(
        admin.approve_manual_admission(&request).await,
        Err(ManualAdmissionError::Rejected)
    ));
    assert_eq!(admin.status().applied_count, 4);
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn material_file_failure_retains_pending_state_and_can_retry_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let join_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::join(&join_path, admin.manual_invitation().unwrap())
        .await
        .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    let original = admin.approve_manual_admission(&request).await.unwrap();
    // A genuine filesystem failure at the private-material destination. The
    // fault is external to the node; no mocked successful storage is involved.
    std::fs::create_dir(join_path.join("admission.material")).unwrap();
    let receiver = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("approved result");
        };
        let failure = joining.complete_manual_admission(delivery).await;
        assert!(matches!(failure, Err(ManualAdmissionError::Node(_))));
        assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    };
    let (_, delivered) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    assert!(matches!(
        delivered,
        Err(ManualAdmissionError::PathUnavailable)
    ));
    let result = admin.approved_admission(&request).await.unwrap().unwrap();
    assert_eq!(result.authorization, original.authorization);
    assert_eq!(result.stage, AdmissionStage::PendingMaterials);
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
    std::fs::remove_dir(join_path.join("admission.material")).unwrap();

    let mut admin = NativeNetwork::open(&admin_path).await.unwrap();
    let mut joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.admission_stage(), AdmissionStage::PendingMaterials);
    assert!(
        !join_path.join("genesis.credential").exists(),
        "an ordinary applicant cannot acquire the root private key"
    );
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let receiver = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("retry approved result");
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (complete, delivered) = tokio::join!(receiver, admin.serve_manual_admission(&server));
    assert_eq!(complete, AdmissionStage::Complete);
    assert_eq!(delivered.unwrap(), AdmissionStage::Complete);
    assert_eq!(admin.status().applied_count, 2);
    assert_eq!(
        admin
            .approved_admission(&request)
            .await
            .unwrap()
            .unwrap()
            .stage,
        AdmissionStage::Complete
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(join_path.join("admission.material"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    let joining = NativeNetwork::open(&join_path).await.unwrap();
    assert_eq!(joining.admission_stage(), AdmissionStage::Complete);
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn authenticated_contact_cannot_make_a_corrupt_signature_durable_membership() {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let anchor = admin.status().anchor;
    let mut joining = NativeNetwork::join(
        dir.path().join("joining"),
        admin.manual_invitation().unwrap(),
    )
    .await
    .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    admin.approve_manual_admission(&request).await.unwrap();
    // Obtain the real signed fixture through the store's public history API;
    // corrupt its signature before sending it on the genuine approved TLS
    // contact. The receiving node must not trust transport authentication as
    // a substitute for the signed authorization verifier.
    let fixture = miopunch_native_probe::management_store::ManagementStore::open(
        admin_path.join("management.sqlite"),
        anchor,
    )
    .await
    .unwrap();
    let records = fixture.applied_records().await.unwrap();
    fixture.close().await;
    let mut raw: Vec<_> = records
        .iter()
        .map(|record| {
            (
                record.header().to_vec(),
                record.body().map(|body| body.to_vec()),
            )
        })
        .collect();
    raw[1].0[10] ^= 1;
    assert!(Header::<ManagementExtension>::decode(&raw[1].0).is_err());
    let damaged = p2panda_core::Hash::digest(&raw[1].0);
    let bytes = p2panda_core::cbor::encode_cbor(&(damaged, raw)).unwrap();
    let invalid_contact = async {
        let (mut send, mut recv) = server.accept_bi().await.unwrap();
        let mut request = [0; 144];
        recv.read_exact(&mut request).await.unwrap();
        send.write_all(&[1]).await.unwrap();
        send.write_all(&0_u64.to_be_bytes()).await.unwrap();
        send.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        send.write_all(&bytes).await.unwrap();
        send.finish().unwrap();
        let mut ack = [0; 4];
        assert!(
            recv.read_exact(&mut ack).await.is_err(),
            "unverified material cannot be acknowledged as saved authorization"
        );
    };
    let (rejected, _) = tokio::join!(joining.request_manual_admission(&client), invalid_contact);
    assert!(matches!(rejected, Err(ManualAdmissionError::Rejected)));
    assert_eq!(joining.admission_stage(), AdmissionStage::AwaitingApproval);
    assert_eq!(joining.status().applied_count, 0);
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}

#[tokio::test]
async fn another_authenticated_request_for_an_existing_member_reuses_its_valid_grant() {
    let dir = tempfile::tempdir().unwrap();
    let mut admin = NativeNetwork::create(dir.path().join("admin"))
        .await
        .unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let mut joining = NativeNetwork::join(dir.path().join("joining"), invitation)
        .await
        .unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let request = admin.manual_requests().await.unwrap().pop().unwrap();
    let original = admin.approve_manual_admission(&request).await.unwrap();
    let other_request = miopunch_native_probe::manual_admission::AdmissionRequest {
        applicant: request.applicant,
        request_id: [36; 16],
    };
    let repeat = async {
        let (mut send, mut recv) = client.open_bi().await.unwrap();
        send.write_all(&request_wire(
            invitation,
            other_request.request_id,
            request.applicant,
        ))
        .await
        .unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(1).await.unwrap(), [0]);
    };
    let (_, received) = tokio::join!(repeat, admin.serve_manual_admission(&server));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let reused = admin
        .approve_manual_admission(&other_request)
        .await
        .unwrap();
    assert_eq!(reused.authorization, original.authorization);
    assert_eq!(reused.stage, AdmissionStage::Approved);
    assert_eq!(admin.status().applied_count, 2);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}

#[test]
fn approval_commit_checkpoint_worker() {
    let Ok(path) = std::env::var("MIO30_APPROVAL_CHECKPOINT") else {
        return;
    };
    let descriptor: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let anchor = miopunch_native_probe::management_auth::NetworkAnchor {
            root: descriptor["root"].as_str().unwrap().parse().unwrap(),
            genesis: descriptor["genesis"].as_str().unwrap().parse().unwrap(),
        };
        let request = miopunch_native_probe::manual_admission::AdmissionRequest {
            applicant: descriptor["applicant"].as_str().unwrap().parse().unwrap(),
            request_id: hex::decode(descriptor["request_id"].as_str().unwrap()).unwrap().try_into().unwrap(),
        };
        let record = miopunch_native_probe::management_store::ManagementRecord::new(hex::decode(descriptor["header"].as_str().unwrap()).unwrap(), None);
        let store = miopunch_native_probe::management_store::ManagementStore::open(descriptor["database"].as_str().unwrap(), anchor).await.unwrap();
        // The actual shared approval transaction commits its signed Add and
        // delivery result together, exactly as in the production node path.
        store.save_admission_approval(&request, &record).await.unwrap();
        eprintln!("actual approval process exits after signed Add/result transaction commit, before application");
        std::process::exit(78);
    });
}

#[tokio::test]
async fn approval_commit_crash_is_recovered_before_another_same_identity_request_decides_to_add() {
    approval_checkpoint_case(true).await;
}

#[tokio::test]
async fn approval_commit_interruption_is_recovered_before_a_live_nodes_next_request_decides_to_add()
{
    approval_checkpoint_case(false).await;
}

async fn approval_checkpoint_case(reopen_before_decision: bool) {
    let dir = tempfile::tempdir().unwrap();
    let admin_path = dir.path().join("admin");
    let join_path = dir.path().join("joining");
    let mut admin = NativeNetwork::create(&admin_path).await.unwrap();
    let invitation = admin.manual_invitation().unwrap();
    let mut joining = NativeNetwork::join(&join_path, invitation).await.unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let (response, received) = tokio::join!(
        joining.request_manual_admission(&client),
        admin.serve_manual_admission(&server)
    );
    assert!(matches!(
        response.unwrap(),
        ManualExchange::AwaitingApproval
    ));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let first = admin.manual_requests().await.unwrap().pop().unwrap();
    let second = miopunch_native_probe::manual_admission::AdmissionRequest {
        applicant: first.applicant,
        request_id: [43; 16],
    };
    let authenticate_second = async {
        let (mut send, mut recv) = client.open_bi().await.unwrap();
        send.write_all(&request_wire(
            invitation,
            second.request_id,
            second.applicant,
        ))
        .await
        .unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(1).await.unwrap(), [0]);
    };
    let (_, received) = tokio::join!(authenticate_second, admin.serve_manual_admission(&server));
    assert_eq!(received.unwrap(), AdmissionStage::AwaitingApproval);
    let key = SigningKey::from_bytes(
        &std::fs::read(admin_path.join("node.key"))
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let header = Header::<ManagementExtension>::builder()
        .build(
            &key,
            ManagementExtension {
                version: 1,
                log_id: rand::random(),
                group: GroupsExtensionArgs {
                    group_id: invitation.anchor.root,
                    action: GroupAction::Add {
                        member: GroupMember::Individual(first.applicant),
                        access: Access::read(),
                    },
                    dependencies: vec![invitation.anchor.genesis],
                },
            },
        )
        .encode();
    let original_grant = p2panda_core::Hash::digest(&header);
    let descriptor = dir.path().join("approval-checkpoint.json");
    std::fs::write(&descriptor, serde_json::to_vec(&serde_json::json!({
        "root": invitation.anchor.root.to_string(), "genesis": invitation.anchor.genesis.to_string(),
        "applicant": first.applicant.to_string(), "request_id": hex::encode(first.request_id),
        "header": hex::encode(header), "database": admin_path.join("management.sqlite"),
    })).unwrap()).unwrap();
    client.close(0u32.into(), b"checkpoint");
    server.close(0u32.into(), b"checkpoint");
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
    let live_admin = if reopen_before_decision {
        None
    } else {
        // Retain the original cached authority view while the real shared
        // transaction commits, as after cancelling an in-process approval.
        Some(NativeNetwork::open(&admin_path).await.unwrap())
    };
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "approval_commit_checkpoint_worker",
            "--nocapture",
        ])
        .env("MIO30_APPROVAL_CHECKPOINT", &descriptor)
        .output()
        .unwrap();
    assert_eq!(
        child.status.code(),
        Some(78),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&child.stderr));
    let mut admin = match live_admin {
        Some(admin) => admin,
        None => NativeNetwork::open(&admin_path).await.unwrap(),
    };
    let reused = admin.approve_manual_admission(&second).await.unwrap();
    eprintln!(
        "MIO30_APPROVAL_RECOVERY applied={} original_grant_reused={}",
        admin.status().applied_count,
        reused.authorization == original_grant
    );
    assert_eq!(
        admin.status().applied_count,
        2,
        "a committed original Add must be recovered before the second request decides membership is absent"
    );
    assert_eq!(reused.authorization, original_grant);
    assert_eq!(
        admin
            .approved_admission(&first)
            .await
            .unwrap()
            .unwrap()
            .authorization,
        original_grant
    );
    let mut joining = NativeNetwork::open(&join_path).await.unwrap();
    let server_endpoint = admin.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let client_endpoint = joining.bind_endpoint(Ipv4Addr::LOCALHOST).await.unwrap();
    let (client, server) =
        connect(&client_endpoint, &server_endpoint, ADMISSION_REQUEST_ALPN).await;
    let receive = async {
        let ManualExchange::Materials(delivery) =
            joining.request_manual_admission(&client).await.unwrap()
        else {
            panic!("original committed result");
        };
        joining.complete_manual_admission(delivery).await.unwrap()
    };
    let (local, remote) = tokio::join!(receive, admin.serve_manual_admission(&server));
    assert_eq!(local, AdmissionStage::Complete);
    assert_eq!(remote.unwrap(), AdmissionStage::Complete);
    assert_eq!(admin.status().applied_count, 2);
    client.close(0u32.into(), b"done");
    server.close(0u32.into(), b"done");
    client_endpoint.close().await;
    server_endpoint.close().await;
    joining.close().await;
    admin.close().await;
}
