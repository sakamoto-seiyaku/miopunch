//! Restricted control must preserve the existing same-version application wire.
//! The counterpart is an independently pinned pre-migration Native SDK executable.
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{
        AdmissionOutcome, ControlConfirmation, ControlConnection, ControlMessageFormat,
        EndpointAdmission, presets,
    },
};
use std::{io, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

#[derive(Debug, Default)]
struct ExistingRequest;
const NONCE: [u8; 16] = [11; 16];
impl EndpointAdmission for ExistingRequest {
    async fn admit(&self, control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let exchange = control.open_control_exchange().await?;
        // Existing miopunch/session/admission/1 MUC1 initiator offer, with no DATA permission.
        let mut offer = [0; 28];
        offer[..4].copy_from_slice(b"MUC1");
        offer[4..11].copy_from_slice(&[1, 2, 1, 1, 1, 0, 0]);
        offer[12..].copy_from_slice(&NONCE);
        let format = ControlMessageFormat::LengthPrefixed16 { maximum: 28 };
        exchange.send_message(format, &offer).await?;
        let peer = exchange.recv_message(format).await?;
        if peer.len() != 28
            || &peer[..4] != b"MUC1"
            || peer[4..8] != [1, 2, 1, 2]
            || peer[9] != 0
            || peer[11] != 0
        {
            return Err(io::Error::other(
                "existing request counterpart sent invalid capability offer",
            ));
        }
        let peer_nonce: [u8; 16] = peer[12..].try_into().unwrap();
        Ok(AdmissionOutcome::ControlOnlyWithConfirmation(
            ControlConfirmation::new(async move {
                let mut confirmation = [0; 17];
                confirmation[0] = 1;
                confirmation[1..].copy_from_slice(&peer_nonce);
                let format = ControlMessageFormat::EndTerminated { maximum: 17 };
                exchange.send_message(format, &confirmation).await?;
                let peer = exchange.recv_message(format).await?;
                if peer.len() != 17 || peer[0] != 1 || peer[1..] != NONCE {
                    return Err(io::Error::other(
                        "existing request counterpart did not confirm this exchange",
                    ));
                }
                Ok(())
            }),
        ))
    }
    fn is_current(&self, _control: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
#[ignore = "requires the pinned pre-migration SDK counterpart recorded in the M93 evidence"]
async fn restricted_control_preserves_the_existing_native_request_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new(
        std::env::var_os("MIOPUNCH_OLD_NATIVE_COUNTERPART")
            .expect("pinned original Native SDK executable"),
    )
    .arg(directory.path().join("counterpart"))
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(ready["event"], "ready");
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .restricted_application(ExistingRequest::default())
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(ready["id"].as_str().unwrap().parse().unwrap())
        .with_ip_addr(ready["address"].as_str().unwrap().parse().unwrap());
    let connection = endpoint
        .connect_control_with_opts(
            target,
            b"miopunch/session/admission/1",
            iroh::endpoint::ConnectOptions::new()
                .with_caller_deadline(n0_future::time::Instant::now() + Duration::from_secs(5)),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    loop {
        let line = lines.next_line().await.unwrap().unwrap();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
            if value["event"] == "control-ready" {
                assert_eq!(value["requestAllowed"], true);
                assert_eq!(value["memberAllowed"], false);
                break;
            }
        }
    }
    assert_eq!(
        connection.remote_id(),
        ready["id"].as_str().unwrap().parse().unwrap()
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"close\n")
        .await
        .unwrap();
    connection.close(0u32.into(), b"restricted interoperability complete");
    drop(connection);
    endpoint.close().await;
    drop(endpoint);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

#[derive(Debug)]
struct PendingConfirmation {
    entered: std::sync::Arc<tokio::sync::Notify>,
    active: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
struct ConfirmationOwner(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl Drop for ConfirmationOwner {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
impl EndpointAdmission for PendingConfirmation {
    async fn admit(&self, _control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        self.active
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let owner = ConfirmationOwner(self.active.clone());
        let entered = self.entered.clone();
        Ok(AdmissionOutcome::ControlOnlyWithConfirmation(
            ControlConfirmation::new(async move {
                let _owner = owner;
                entered.notify_one();
                std::future::pending::<io::Result<()>>().await
            }),
        ))
    }
    fn is_current(&self, _control: &ControlConnection) -> bool {
        true
    }
}

#[tokio::test]
async fn cancellation_releases_both_attempts_owned_application_confirmations() {
    let active_a = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active_b = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let entered_a = std::sync::Arc::new(tokio::sync::Notify::new());
    let entered_b = std::sync::Arc::new(tokio::sync::Notify::new());
    let a = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .alpns(vec![b"owned-confirmation".to_vec()])
        .restricted_application(PendingConfirmation {
            entered: entered_a.clone(),
            active: active_a.clone(),
        })
        .bind()
        .await
        .unwrap();
    let b = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .restricted_application(PendingConfirmation {
            entered: entered_b.clone(),
            active: active_b.clone(),
        })
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(a.id())
        .with_ip_addr(a.addr().ip_addrs().find(|a| a.is_ipv4()).copied().unwrap());
    let cancel = tokio_util::sync::CancellationToken::new();
    let options = iroh::endpoint::ConnectOptions::new().with_attempt_cancellation(cancel.clone());
    let incoming = tokio::spawn({
        let a = a.clone();
        async move { a.accept().await.unwrap().accept_control().unwrap().await }
    });
    let outgoing = tokio::spawn({
        let b = b.clone();
        async move {
            b.connect_control_with_opts(target, b"owned-confirmation", options)
                .await
                .unwrap()
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(entered_a.notified(), entered_b.notified());
    })
    .await
    .unwrap();
    cancel.cancel();
    let results = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(outgoing, incoming)
    })
    .await
    .unwrap();
    assert!(results.0.unwrap().is_err() && results.1.unwrap().is_err());
    // Endpoints are deliberately still owned: cancellation must drop per-attempt state.
    assert_eq!(active_a.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(active_b.load(std::sync::atomic::Ordering::SeqCst), 0);
    tokio::join!(a.close(), b.close());
}

#[derive(Debug)]
struct NoConfirmation;
impl EndpointAdmission for NoConfirmation {
    async fn admit(&self, _control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        Ok(AdmissionOutcome::ControlOnly)
    }
    fn is_current(&self, _control: &ControlConnection) -> bool {
        true
    }
}
#[tokio::test]
async fn application_mode_does_not_invent_a_missing_delivery_agreement() {
    let a = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .alpns(vec![b"missing-confirmation".to_vec()])
        .restricted_application(NoConfirmation)
        .bind()
        .await
        .unwrap();
    let b = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .restricted_application(NoConfirmation)
        .bind()
        .await
        .unwrap();
    let target = EndpointAddr::new(a.id())
        .with_ip_addr(a.addr().ip_addrs().find(|a| a.is_ipv4()).copied().unwrap());
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(b.connect_control(target, b"missing-confirmation"), async {
            a.accept().await.unwrap().accept_control().unwrap().await
        })
    })
    .await
    .unwrap();
    assert!(result.0.is_err() && result.1.is_err());
    tokio::join!(a.close(), b.close());
}

#[derive(Debug)]
struct OwnedControlWorker;
impl EndpointAdmission for OwnedControlWorker {
    async fn admit(&self, control: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let handle = control.control_handle();
        let original = control.stable_id();
        tokio::spawn(async move {
            let exchange = if handle.side().is_client() {
                handle.open_control_exchange().await?
            } else {
                handle.accept_control_exchange().await?
            };
            let format = ControlMessageFormat::EndTerminated { maximum: 7 };
            exchange.send_message(format, b"CONTROL").await?;
            if exchange.recv_message(format).await? != b"CONTROL" || handle.stable_id() != original
            {
                return Err(io::Error::other(
                    "control worker changed session or message",
                ));
            }
            Ok::<_, io::Error>(())
        })
        .await
        .map_err(io::Error::other)??;
        Ok(AdmissionOutcome::ControlOnlyWithConfirmation(
            ControlConfirmation::new(async { Ok(()) }),
        ))
    }
    fn is_current(&self, _control: &ControlConnection) -> bool {
        true
    }
}
#[tokio::test]
async fn owned_control_workers_preserve_original_session_and_owner_drop_still_closes() {
    let builder = || {
        Endpoint::builder(presets::Minimal)
            .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
            .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
            .unwrap()
            .alpns(vec![b"owned-control-worker".to_vec()])
            .restricted_application(OwnedControlWorker)
    };
    let a = builder().bind().await.unwrap();
    let b = builder().bind().await.unwrap();
    let target = EndpointAddr::new(a.id())
        .with_ip_addr(a.addr().ip_addrs().find(|a| a.is_ipv4()).copied().unwrap());
    let (client, server) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(b.connect_control(target, b"owned-control-worker"), async {
            a.accept().await.unwrap().accept_control().unwrap().await
        })
    })
    .await
    .unwrap();
    let client = client.unwrap();
    let server = server.unwrap();
    let client_handle = client.control_handle();
    let server_handle = server.control_handle();
    assert_eq!(client_handle.stable_id(), client.stable_id());
    assert_eq!(server_handle.stable_id(), server.stable_id());
    drop((client, server));
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(client_handle.closed(), server_handle.closed())
    })
    .await;
    let exchange_failed = client_handle.open_control_exchange().await.is_err()
        && server_handle.open_control_exchange().await.is_err();
    tokio::join!(a.close(), b.close());
    assert!(
        closed.is_ok() && exchange_failed,
        "retained control helpers cannot outlive their admission owner"
    );
}
