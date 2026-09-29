use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use miopunch_native_probe::admission_proof::{
    AdmissionScope, AdmissionSecret, ProofError, prove_client, prove_server,
};
use rsasl::callback::{Context, Request, SessionCallback, SessionData};
use rsasl::config::SASLConfig;
use rsasl::mechanisms::scram::SCRAM_SHA256_PLUS;
use rsasl::prelude::{ChannelBindingCallback, Registry, SASLClient, SessionError, State};
use rsasl::property::{AuthId, OverrideCBType, Password};

const ALPN: &[u8] = b"miopunch/admission-proof/1";

async fn pair() -> (
    Endpoint,
    Endpoint,
    iroh::endpoint::Connection,
    iroh::endpoint::Connection,
) {
    let server = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let addr = server.addr();
    let target = EndpointAddr::new(server.id())
        .with_addrs(addr.ip_addrs().copied().map(iroh_base::TransportAddr::Ip));
    let connecting = client.connect(target, ALPN);
    let accepting = async { server.accept().await.unwrap().await.unwrap() };
    let (out, incoming) = tokio::join!(connecting, accepting);
    (client, server, out.unwrap(), incoming)
}

#[tokio::test]
async fn correct_secret_proves_both_sides_on_real_tls() {
    let (client_endpoint, server_endpoint, client, server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let client_secret = AdmissionSecret::from_saved([11; 32]);
    let server_secret = AdmissionSecret::from_saved([11; 32]);
    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&client, &scope, &client_secret),
            prove_server(&server, &scope, &server_secret)
        )
    })
    .await
    .unwrap();
    assert!(client_result.is_ok(), "{client_result:?}");
    assert!(server_result.is_ok(), "{server_result:?}");
    let client_fact = client_result.unwrap();
    let server_fact = server_result.unwrap();
    assert_eq!(client_fact.scope(), &scope);
    assert_eq!(server_fact.scope(), &scope);
    assert_eq!(client_fact.peer(), server_endpoint.id());
    assert_eq!(server_fact.peer(), client_endpoint.id());
    assert!(client_fact.applies_to(&client, &scope));
    assert!(server_fact.applies_to(&server, &scope));
    assert!(!client_fact.applies_to(&server, &scope));
    assert!(!client_fact.applies_to(&client, &AdmissionScope::new([7; 32], 5)));
    assert!(matches!(
        prove_client(&client, &scope, &client_secret).await,
        Err(ProofError::AlreadyAttempted)
    ));
    assert!(matches!(
        prove_server(&server, &scope, &server_secret).await,
        Err(ProofError::AlreadyAttempted)
    ));
    client_fact.invalidate();
    assert!(client.close_reason().is_some());
    assert!(!client_fact.applies_to(&client, &scope));
    drop(server_fact);
    assert!(server.close_reason().is_some());
    client_endpoint.close().await;
    server_endpoint.close().await;
}

async fn read_frame(recv: &mut iroh::endpoint::RecvStream) -> Vec<u8> {
    let mut len = [0; 2];
    recv.read_exact(&mut len).await.unwrap();
    let mut bytes = vec![0; u16::from_be_bytes(len) as usize];
    recv.read_exact(&mut bytes).await.unwrap();
    bytes
}

async fn maybe_read_frame(recv: &mut iroh::endpoint::RecvStream) -> Option<Vec<u8>> {
    let mut len = [0; 2];
    recv.read_exact(&mut len).await.ok()?;
    let mut bytes = vec![0; u16::from_be_bytes(len) as usize];
    recv.read_exact(&mut bytes).await.ok()?;
    Some(bytes)
}

async fn write_frame(send: &mut iroh::endpoint::SendStream, bytes: &[u8]) {
    send.write_all(&(bytes.len() as u16).to_be_bytes())
        .await
        .unwrap();
    send.write_all(bytes).await.unwrap();
}

#[tokio::test]
async fn wrong_secret_network_and_secret_generation_do_not_prove_admission() {
    let scope = AdmissionScope::new([7; 32], 4);
    for (server_scope, server_secret_bytes) in [
        (scope.clone(), [12; 32]),
        (AdmissionScope::new([7; 32], 5), [11; 32]),
        (AdmissionScope::new([8; 32], 4), [11; 32]),
    ] {
        let (_client_endpoint, _server_endpoint, client, server) = pair().await;
        let client_secret = AdmissionSecret::from_saved([11; 32]);
        let server_secret = AdmissionSecret::from_saved(server_secret_bytes);
        let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(
                prove_client(&client, &scope, &client_secret),
                prove_server(&server, &server_scope, &server_secret)
            )
        })
        .await
        .unwrap();
        assert!(client_result.is_err());
        assert!(matches!(server_result, Err(ProofError::Rejected)));
        assert!(client.close_reason().is_some());
        assert!(server.close_reason().is_some());
    }
}

#[tokio::test]
async fn cancelling_a_pending_proof_closes_its_connection() {
    let (_client_endpoint, _server_endpoint, _client, server) = pair().await;
    let task_connection = server.clone();
    let task = tokio::spawn(async move {
        prove_server(
            &task_connection,
            &AdmissionScope::new([7; 32], 4),
            &AdmissionSecret::from_saved([11; 32]),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    task.abort();
    let _ = task.await;
    assert!(server.close_reason().is_some());
}

async fn relay(client_side: &iroh::endpoint::Connection, server_side: &iroh::endpoint::Connection) {
    let (mut send_a, mut recv_a) = client_side.accept_bi().await.unwrap();
    let (mut send_b, mut recv_b) = server_side.open_bi().await.unwrap();
    let first = read_frame(&mut recv_a).await;
    write_frame(&mut send_b, &first).await;
    let server_first = read_frame(&mut recv_b).await;
    write_frame(&mut send_a, &server_first).await;
    let client_final = read_frame(&mut recv_a).await;
    write_frame(&mut send_b, &client_final).await;
    if let Some(server_final) = maybe_read_frame(&mut recv_b).await {
        let _ = send_a
            .write_all(&(server_final.len() as u16).to_be_bytes())
            .await;
        let _ = send_a.write_all(&server_final).await;
    }
}

#[tokio::test]
async fn proof_forwarded_to_a_different_tls_connection_fails() {
    let (_a_client_endpoint, _a_relay_endpoint, a_client, a_relay) = pair().await;
    let (_b_relay_endpoint, _b_server_endpoint, b_relay, b_server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let secret = AdmissionSecret::from_saved([11; 32]);
    let (a, b, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            prove_client(&a_client, &scope, &secret),
            prove_server(&b_server, &scope, &secret),
            relay(&a_relay, &b_relay)
        )
    })
    .await
    .unwrap();
    assert!(a.is_err(), "client accepted forwarded proof: {a:?}");
    assert!(b.is_err(), "server accepted forwarded proof: {b:?}");
}

#[tokio::test]
async fn changed_server_final_is_not_mutual_success() {
    let (_client_endpoint, _server_endpoint, client, server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let secret = AdmissionSecret::from_saved([11; 32]);
    let malicious = async {
        let (mut send, mut recv) = server.accept_bi().await.unwrap();
        let first = String::from_utf8(read_frame(&mut recv).await).unwrap();
        let nonce = first.split(",r=").nth(1).unwrap();
        let server_first =
            format!("r={nonce}SERVERNONCE,s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=,i=16384");
        write_frame(&mut send, server_first.as_bytes()).await;
        let _ = read_frame(&mut recv).await;
        write_frame(&mut send, b"v=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(prove_client(&client, &scope, &secret), malicious)
    })
    .await
    .unwrap();
    assert!(result.is_err(), "client accepted changed final: {result:?}");
}

#[tokio::test]
async fn unbound_or_downgraded_first_message_is_rejected() {
    for first in [
        b"n,,n=ignored,r=nonce".as_slice(),
        b"y,,n=ignored,r=nonce",
        b"p=tls-unique,,n=ignored,r=nonce",
        b"p=tls-exporter,a=admin,n=ignored,r=nonce",
    ] {
        let (_client_endpoint, _server_endpoint, client, server) = pair().await;
        let scope = AdmissionScope::new([7; 32], 4);
        let secret = AdmissionSecret::from_saved([11; 32]);
        let server_result = prove_server(&server, &scope, &secret);
        let malicious = async {
            let (mut send, _recv) = client.open_bi().await.unwrap();
            write_frame(&mut send, first).await;
        };
        let (result, ()) = tokio::join!(server_result, malicious);
        assert!(result.is_err(), "accepted {first:?}");
    }
}

#[tokio::test]
async fn untrusted_iteration_count_is_rejected_before_expensive_work() {
    let (_client_endpoint, _server_endpoint, client, server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let secret = AdmissionSecret::from_saved([11; 32]);
    let malicious = async {
        let (mut send, mut recv) = server.accept_bi().await.unwrap();
        let _ = read_frame(&mut recv).await;
        write_frame(
            &mut send,
            b"r=nonce,s=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,i=4294967295",
        )
        .await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(prove_client(&client, &scope, &secret), malicious)
    })
    .await
    .unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn exporter_is_equal_at_both_tls_ends_and_changes_on_reconnect() {
    let (_client_endpoint, _server_endpoint, client, server) = pair().await;
    let mut a = [0; 32];
    let mut b = [0; 32];
    client
        .export_keying_material(&mut a, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    server
        .export_keying_material(&mut b, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    assert_eq!(a, b);
    let (_client_endpoint2, _server_endpoint2, client2, server2) = pair().await;
    let mut c = [0; 32];
    let mut d = [0; 32];
    client2
        .export_keying_material(&mut c, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    server2
        .export_keying_material(&mut d, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    assert_eq!(c, d);
    assert_ne!(a, c);
}

struct TestCredentials {
    auth_id: String,
    password: String,
}
impl SessionCallback for TestCredentials {
    fn callback(
        &self,
        _: &SessionData,
        _: &Context,
        request: &mut Request,
    ) -> Result<(), SessionError> {
        request
            .satisfy::<AuthId>(&self.auth_id)?
            .satisfy::<Password>(self.password.as_bytes())?
            .satisfy::<OverrideCBType>("tls-exporter")?;
        Ok(())
    }
    fn enable_channel_binding(&self) -> bool {
        true
    }
}
struct TestBinding([u8; 32]);
impl ChannelBindingCallback for TestBinding {
    fn get_cb_data(&self, name: &str) -> Option<&[u8]> {
        (name == "tls-exporter").then_some(&self.0)
    }
}

async fn manual_client(
    client: &iroh::endpoint::Connection,
    exporter: [u8; 32],
) -> (Vec<u8>, Vec<u8>, bool) {
    static MECHANISMS: &[rsasl::prelude::Mechanism] = &[SCRAM_SHA256_PLUS];
    let config = SASLConfig::builder()
        .with_registry(Registry::with_mechanisms(MECHANISMS))
        .with_callback(TestCredentials {
            auth_id: format!("miopunch:{}:4", hex::encode([7; 32])),
            password: hex::encode([11; 32]),
        })
        .unwrap();
    let mut session = SASLClient::with_cb(config, TestBinding(exporter))
        .start_suggested(&[SCRAM_SHA256_PLUS.mechanism])
        .unwrap();
    let (mut send, mut recv) = client.open_bi().await.unwrap();
    let mut first = Vec::new();
    assert!(session.step(None, &mut first).unwrap().is_running());
    write_frame(&mut send, &first).await;
    let server_first = read_frame(&mut recv).await;
    let mut client_final = Vec::new();
    assert!(
        session
            .step(Some(&server_first), &mut client_final)
            .unwrap()
            .is_running()
    );
    write_frame(&mut send, &client_final).await;
    let Some(server_final) = maybe_read_frame(&mut recv).await else {
        return (first, client_final, false);
    };
    let mut unused = Vec::new();
    let verified = matches!(
        session.step(Some(&server_final), &mut unused),
        Ok(State::Finished(_))
    );
    if verified {
        write_frame(&mut send, b"OK").await;
        assert_eq!(read_frame(&mut recv).await, b"OK");
    }
    (first, client_final, verified)
}

#[tokio::test]
async fn wrong_exporter_value_on_a_real_tls_connection_fails() {
    let (_client_endpoint, _server_endpoint, client, server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let secret = AdmissionSecret::from_saved([11; 32]);
    let mut wrong_binding = [0; 32];
    client
        .export_keying_material(&mut wrong_binding, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    wrong_binding[0] ^= 1;
    let (client_result, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            manual_client(&client, wrong_binding),
            prove_server(&server, &scope, &secret)
        )
    })
    .await
    .unwrap();
    assert!(!client_result.2);
    assert!(matches!(server_result, Err(ProofError::Rejected)));
}

#[tokio::test]
async fn previous_valid_client_transcript_cannot_be_replayed_on_a_new_connection() {
    let (_client_endpoint, _server_endpoint, client, server) = pair().await;
    let scope = AdmissionScope::new([7; 32], 4);
    let secret = AdmissionSecret::from_saved([11; 32]);
    let mut exporter = [0; 32];
    client
        .export_keying_material(&mut exporter, b"EXPORTER-Channel-Binding", b"")
        .unwrap();
    let (transcript, server_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(
            manual_client(&client, exporter),
            prove_server(&server, &scope, &secret)
        )
    })
    .await
    .unwrap();
    assert!(transcript.2);
    assert!(server_result.is_ok());
    let encoded_secret = hex::encode([11; 32]);
    for message in [&transcript.0, &transcript.1] {
        assert!(!message.windows(32).any(|part| part == [11; 32]));
        assert!(
            !message
                .windows(64)
                .any(|part| part == encoded_secret.as_bytes())
        );
    }

    let (_client_endpoint2, _server_endpoint2, replay_client, replay_server) = pair().await;
    let replay = async {
        let (mut send, mut recv) = replay_client.open_bi().await.unwrap();
        write_frame(&mut send, &transcript.0).await;
        let _new_server_first = read_frame(&mut recv).await;
        write_frame(&mut send, &transcript.1).await;
        if let Some(final_message) = maybe_read_frame(&mut recv).await {
            assert!(final_message.starts_with(b"e="), "{final_message:?}");
        }
    };
    let ((), replay_result) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(replay, prove_server(&replay_server, &scope, &secret))
    })
    .await
    .unwrap();
    assert!(replay_result.is_err());
}
