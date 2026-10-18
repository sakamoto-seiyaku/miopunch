//! A control session releases the original connection only through real Core admission.
#[path = "support/control_handoff.rs"]
mod handoff;
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{ControlMessageFormat, presets},
};
use iroh_base::SecretKey;
use miopunch_native_probe::{
    admission_gate::{AdmissionGate, CONTROL_ALPN},
    admission_proof::{AdmissionScope, AdmissionSecret},
};
use std::{io, net::Ipv4Addr, time::Duration};

#[tokio::test]
async fn prepared_control_releases_original_member_and_queued_business() {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut pair = Pair::new().await;
        let (client_pending, server_pending, dial, accept) = pair.pending().await;
        let client_control = client_pending.control.clone();
        let server_control = server_pending.control.clone();
        let ids = (client_control.stable_id(), server_control.stable_id());
        let (sent, received) = tokio::join!(
            async {
                let exchange = client_control.open_control_exchange().await.unwrap();
                exchange
                    .send_message(ControlMessageFormat::EndTerminated { maximum: 4 }, b"PATH")
                    .await
                    .unwrap();
            },
            async {
                let exchange = server_control.accept_control_exchange().await.unwrap();
                assert_eq!(
                    exchange
                        .recv_message(ControlMessageFormat::EndTerminated { maximum: 4 })
                        .await
                        .unwrap(),
                    b"PATH"
                );
            }
        );
        let _ = (sent, received);
        let mut client_session = client_pending.session(dial);
        let mut server_session = server_pending.session(accept);
        client_session.queue_business(b"PING".to_vec()).unwrap();
        server_session.queue_business(b"PONG".to_vec()).unwrap();
        let (client_data, server_data) = tokio::join!(
            client_session.commit_data_path(),
            server_session.commit_data_path()
        );
        let mut client_data = client_data.unwrap();
        let mut server_data = server_data.unwrap();
        assert_eq!(
            (
                client_data.connection().stable_id(),
                server_data.connection().stable_id()
            ),
            ids
        );
        let ping = client_data.take_pending_business().pop_front().unwrap();
        let pong = server_data.take_pending_business().pop_front().unwrap();
        let (mut send, mut recv) = client_data.connection().open_bi().await.unwrap();
        send.write_all(&ping).await.unwrap();
        send.finish().unwrap();
        let (mut reply, mut request) = server_data.connection().accept_bi().await.unwrap();
        assert_eq!(request.read_to_end(4).await.unwrap(), b"PING");
        reply.write_all(&pong).await.unwrap();
        reply.finish().unwrap();
        assert_eq!(recv.read_to_end(4).await.unwrap(), b"PONG");
        assert!(client_control.open_control_exchange().await.is_err());
        assert!(server_control.open_control_exchange().await.is_err());
        client_data
            .connection()
            .close(0u32.into(), b"handoff complete");
        pair.close().await;
    })
    .await
    .unwrap();
}

struct Pair {
    client: iroh::endpoint::RestrictedEndpoint,
    server: iroh::endpoint::RestrictedEndpoint,
    receivers: Vec<tokio::sync::mpsc::UnboundedReceiver<handoff::PendingControl>>,
}
impl Pair {
    async fn new() -> Self {
        let keys = [SecretKey::generate(), SecretKey::generate()];
        let members = keys
            .iter()
            .map(|key| *key.public().as_bytes())
            .collect::<Vec<_>>();
        let mut endpoints = Vec::new();
        let mut receivers = Vec::new();
        for key in keys {
            let gate = AdmissionGate::new(
                key.public(),
                AdmissionScope::new([37; 32], 0),
                Some(AdmissionSecret::from_saved([19; 32])),
            );
            gate.update_authorization(members.clone(), []);
            let (pending, receive) = tokio::sync::mpsc::unbounded_channel();
            receivers.push(receive);
            let endpoint = Endpoint::builder(presets::Minimal)
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .secret_key(key)
                .alpns(vec![CONTROL_ALPN.to_vec()])
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .restricted_application(handoff::PendingDataPolicy { gate, pending })
                .bind()
                .await
                .unwrap();
            endpoints.push(endpoint);
        }
        let server = endpoints.pop().unwrap();
        let client = endpoints.pop().unwrap();
        Self {
            client,
            server,
            receivers,
        }
    }
    async fn pending(
        &mut self,
    ) -> (
        handoff::PendingControl,
        handoff::PendingControl,
        tokio::task::JoinHandle<io::Result<iroh::endpoint::Connection>>,
        tokio::task::JoinHandle<io::Result<iroh::endpoint::Connection>>,
    ) {
        let client = &self.client;
        let server = &self.server;
        let receivers = &mut self.receivers;
        let target = EndpointAddr::new(server.id()).with_ip_addr(
            server
                .addr()
                .ip_addrs()
                .find(|a| a.is_ipv4())
                .copied()
                .unwrap(),
        );
        let c = client.clone();
        let s = server.clone();
        let dial = tokio::spawn(async move {
            c.connect(target, CONTROL_ALPN)
                .await
                .map_err(io::Error::other)
        });
        let accept =
            tokio::spawn(async move { s.accept().await.unwrap().await.map_err(io::Error::other) });
        let server_pending = receivers[1].recv().await.unwrap();
        let client_pending = receivers[0].recv().await.unwrap();
        (client_pending, server_pending, dial, accept)
    }
    async fn close(self) {
        tokio::join!(self.client.close(), self.server.close());
    }
}

#[tokio::test]
async fn another_admitted_connection_to_same_peer_cannot_replace_original() {
    use miopunch_native_probe::control_session::{CommitError, ControlSession};
    tokio::time::timeout(Duration::from_secs(8),async {
        let mut pair=Pair::new().await;
        let (c,s,dial,accept)=pair.pending().await;
        let (other_c,other_s,other_dial,other_accept)=pair.pending().await;
        let (other_c,other_s)=tokio::join!(other_c.session(other_dial).commit_data_path(),other_s.session(other_accept).commit_data_path());
        let other_c=other_c.unwrap();let other_s=other_s.unwrap();
        let original=c.control.clone();let wrong=other_c.connection().clone();
        assert_eq!(original.remote_id(),wrong.remote_id());assert_ne!(original.stable_id(),wrong.stable_id());
        let task=handoff::OwnedConnectionTask(dial);
        let session=ControlSession::with_data_handoff(c.control.clone(),move |_| async move {
            let _owned=task;Ok(wrong)
        });
        assert!(matches!(session.commit_data_path().await,Err(CommitError::Terminal {error}) if error.kind()==io::ErrorKind::PermissionDenied));
        assert!(original.close_reason().is_some());assert!(other_c.connection().close_reason().is_some());
        tokio::time::timeout(Duration::from_secs(1),other_s.connection().closed()).await.unwrap();
        assert!(!pair.client.is_closed());assert!(!pair.server.is_closed());
        accept.abort();let _=accept.await;drop((c,s,other_c,other_s));pair.close().await;
    }).await.unwrap();
}

#[tokio::test]
async fn dropping_pending_session_closes_original_and_cancels_owned_accept() {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut pair = Pair::new().await;
        let (c, s, dial, accept) = pair.pending().await;
        let original = c.control.clone();
        let remote = s.control.clone();
        let dial_abort = dial.abort_handle();
        let session = c.session(dial);
        drop(session);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if dial_abort.is_finished() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(original.close_reason().is_some());
        tokio::time::timeout(Duration::from_secs(1), remote.closed())
            .await
            .unwrap();
        assert!(!pair.client.is_closed());
        assert!(!pair.server.is_closed());
        let _ = accept.await;
        drop(s);
        pair.close().await;
    })
    .await
    .unwrap();
}
