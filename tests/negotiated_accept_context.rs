//! Actual SDK admission/hooks around reliable negotiated ALPN and original context.
//! Owned loopback only; no public facility, device or RTC qualification.
use iroh::{
    Endpoint,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks},
};
use miopunch_native_probe::{
    admission_gate::{CONTROL_ALPN, ProtocolScope},
    connection_coordinator::{AttemptContext, ConnectError, MEMBER_ALPN, SessionPurpose},
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
#[path = "support/negotiated_accept_members.rs"]
mod members;

#[derive(Clone, Debug)]
struct Observed {
    deadline: Option<n0_future::time::Instant>,
    token: Option<CancellationToken>,
    original: iroh::endpoint::WeakConnectionHandle,
}
#[derive(Clone, Debug, Default)]
struct Capture {
    hook_calls: Arc<AtomicUsize>,
    observed: Arc<Mutex<Option<Observed>>>,
    blocked: Arc<tokio::sync::Notify>,
    block_member: bool,
}
impl EndpointHooks for Capture {
    async fn after_handshake(&self, connection: &Connection) -> AfterHandshakeOutcome {
        self.hook_calls.fetch_add(1, Ordering::SeqCst);
        *self.observed.lock().unwrap() = Some(Observed {
            deadline: connection.caller_deadline(),
            token: connection.attempt_cancellation(),
            original: connection.weak_handle(),
        });
        if self.block_member && connection.alpn() == MEMBER_ALPN {
            self.blocked.notify_one();
            connection.closed().await;
            return AfterHandshakeOutcome::Reject {
                error_code: 0u32.into(),
                reason: b"owned pending member ended".to_vec(),
            };
        }
        AfterHandshakeOutcome::Accept
    }
}
fn context() -> AttemptContext {
    AttemptContext::new(
        n0_future::time::Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
    )
}
async fn closed_sockets(sockets: &[SocketAddr]) -> serde_json::Value {
    let immediate = members::socket_release(sockets);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let late = members::socket_release(sockets);
    assert!(late.iter().all(|value| *value));
    serde_json::json!({"udpImmediateEmpty":immediate,"udpLate100MsEmpty":late})
}

#[tokio::test]
async fn fragmented_clienthello_is_classified_by_negotiated_alpn_before_original_context_hooks() {
    let (_directory, a, b) = members::admitted_pair().await;
    let capture = Capture::default();
    let server = b
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .hooks(capture.clone()),
        )
        .await
        .unwrap();
    let client = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let sockets: Vec<_> = server
        .addr()
        .ip_addrs()
        .chain(client.addr().ip_addrs())
        .copied()
        .collect();
    // Only the actual strict member ALPN is supported by the server. Unsupported
    // long fixture names enlarge the real ClientHello across Initial packets;
    // none is an accepted protocol version or a compatibility fallback.
    let extras: Vec<_> = (0..48)
        .map(|index| format!("owned-fragment-{index:02}-{}", "x".repeat(200)).into_bytes())
        .collect();
    let original = context();
    let deadline = original.deadline;
    let token = original.cancellation.clone();
    let options = iroh::endpoint::ConnectOptions::new()
        .with_additional_alpns(extras)
        .with_caller_deadline(deadline)
        .with_attempt_cancellation(token.clone());
    let endpoint = client.endpoint().clone();
    let target = server.addr();
    let outbound = tokio::spawn(async move {
        endpoint
            .connect_with_opts(target, MEMBER_ALPN, options)
            .await
            .unwrap()
            .await
    });
    let incoming = server.endpoint().accept().await.unwrap();
    let initial_alpns_available = incoming.initial_alpns().is_some();
    assert!(
        !initial_alpns_available,
        "actual fragmented Initial must make best-effort ALPN unavailable"
    );
    let mut accepting = incoming.accept().unwrap();
    assert_eq!(accepting.alpn().await.unwrap(), MEMBER_ALPN);
    assert_eq!(
        capture.hook_calls.load(Ordering::SeqCst),
        0,
        "classification must not run framework hooks"
    );
    let coordinator = server.coordinator();
    let inbound = coordinator
        .accept_accepting(accepting, SessionPurpose::MemberBusiness, original)
        .await
        .unwrap();
    let outbound = tokio::time::timeout(Duration::from_secs(3), outbound)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(coordinator);
    let observed = capture.observed.lock().unwrap().clone().unwrap();
    assert_eq!(observed.deadline, Some(deadline));
    assert!(observed.token.is_some());
    assert!(a.admission_gate().allows(&outbound, ProtocolScope::Member));
    assert!(b.admission_gate().allows(&inbound, ProtocolScope::Member));
    members::streams(&outbound, &inbound).await;
    token.cancel();
    assert!(
        observed.token.as_ref().unwrap().is_cancelled(),
        "hooks must receive the exact original cancellation source"
    );
    outbound.close(0u32.into(), b"owned negotiated story ended");
    inbound.close(0u32.into(), b"owned negotiated story ended");
    drop((outbound, inbound, observed));
    tokio::join!(client.close(), server.close());
    tokio::join!(a.close(), b.close());
    let cleanup = closed_sockets(&sockets).await;
    println!(
        "NEGOTIATED_ACCEPT_RECEIPT {}",
        serde_json::json!({"actualFragmentedInitial":true,"initialAlpnAvailable":false,
        "reliableNegotiatedMemberAlpn":true,"hooksBeforeDequeue":0,"originalDeadlineMatched":true,"originalTokenMatched":true,
        "savedReopenedMembers":2,"ordinaryHandle":true,"streamMatched":true,"cleanup":cleanup})
    );
}

#[tokio::test]
async fn mismatched_negotiated_purpose_rejects_before_hooks_and_preserves_healthy_member() {
    let (_directory, a, b) = members::admitted_pair().await;
    let healthy_a = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_b = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_ac = healthy_a.coordinator();
    let healthy_bc = healthy_b.coordinator();
    let (healthy, healthy_peer) = tokio::join!(
        healthy_ac.connect(healthy_b.addr(), SessionPurpose::MemberBusiness, context()),
        async {
            healthy_bc
                .accept(
                    healthy_b.endpoint().accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    context(),
                )
                .await
        }
    );
    let (healthy, healthy_peer) = (healthy.unwrap(), healthy_peer.unwrap());
    let capture = Capture::default();
    let pending = b
        .bind_configured_endpoint_with(
            Endpoint::builder(iroh::endpoint::presets::Minimal)
                .bind_addr((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .hooks(capture.clone()),
        )
        .await
        .unwrap();
    // This direct endpoint has no Router to advertise its test control handler.
    // Negotiate the gate's existing control ALPN before testing expected-purpose
    // rejection; a TLS no-application-protocol failure cannot cover that seam.
    pending
        .endpoint()
        .set_alpns(vec![CONTROL_ALPN.to_vec(), MEMBER_ALPN.to_vec()]);
    let pending_client = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let sockets: Vec<_> = pending
        .addr()
        .ip_addrs()
        .chain(pending_client.addr().ip_addrs())
        .copied()
        .collect();
    let endpoint = pending_client.endpoint().clone();
    let target = pending.addr();
    let caller = tokio::spawn(async move { endpoint.connect_control(target, CONTROL_ALPN).await });
    let mut accepting = pending.endpoint().accept().await.unwrap().accept().unwrap();
    assert_eq!(accepting.alpn().await.unwrap(), CONTROL_ALPN);
    let coordinator = pending.coordinator();
    let result = coordinator
        .accept_accepting(accepting, SessionPurpose::MemberBusiness, context())
        .await;
    assert!(matches!(result, Err(ConnectError::ProtocolUnsupported)));
    drop(coordinator);
    assert_eq!(
        capture.hook_calls.load(Ordering::SeqCst),
        0,
        "wrong purpose must run no gate/data/candidate hook"
    );
    assert!(capture.observed.lock().unwrap().is_none());
    assert!(
        tokio::time::timeout(Duration::from_secs(3), caller)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(healthy.close_reason().is_none());
    members::streams(&healthy, &healthy_peer).await;
    tokio::join!(pending.close(), pending_client.close());
    let pending_cleanup = closed_sockets(&sockets).await;
    healthy.close(0u32.into(), b"owned healthy member ended");
    healthy_peer.close(0u32.into(), b"owned healthy member ended");
    drop((healthy, healthy_peer, healthy_ac, healthy_bc));
    tokio::join!(healthy_a.close(), healthy_b.close());
    tokio::join!(a.close(), b.close());
    println!(
        "NEGOTIATED_MISMATCH_RECEIPT {}",
        serde_json::json!({"actualNegotiatedControlAlpn":true,"expectedMemberRejected":true,
        "frameworkHookCalls":0,"candidateHooksEntered":false,"ordinaryHandle":false,"healthyMemberStreamMatched":true,"pendingCleanup":pending_cleanup})
    );
}

#[tokio::test]
async fn original_accept_context_cancel_deadline_and_incoming_token_close_only_pending_m() {
    let (_directory, a, b) = members::admitted_pair().await;
    let healthy_a = a
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_b = b
        .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
        .await
        .unwrap();
    let healthy_ac = healthy_a.coordinator();
    let healthy_bc = healthy_b.coordinator();
    let (healthy, healthy_peer) = tokio::join!(
        healthy_ac.connect(healthy_b.addr(), SessionPurpose::MemberBusiness, context()),
        async {
            healthy_bc
                .accept(
                    healthy_b.endpoint().accept().await.unwrap(),
                    SessionPurpose::MemberBusiness,
                    context(),
                )
                .await
        }
    );
    let (healthy, healthy_peer) = (healthy.unwrap(), healthy_peer.unwrap());
    for kind in ["accepting-cancel", "accepting-deadline", "incoming-token"] {
        let capture = Capture {
            block_member: true,
            ..Capture::default()
        };
        let pending = b
            .bind_configured_endpoint_with(
                Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .bind_addr((Ipv4Addr::LOCALHOST, 0))
                    .unwrap()
                    .hooks(capture.clone()),
            )
            .await
            .unwrap();
        let pending_client = a
            .bind_configured_endpoint(Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let server_token = CancellationToken::new();
        let original_deadline = n0_future::time::Instant::now() + Duration::from_secs(2);
        let original = AttemptContext::new(original_deadline, server_token.clone());
        let address = pending.addr();
        let client_coordinator = pending_client.coordinator();
        let outgoing = tokio::spawn(async move {
            client_coordinator
                .connect(address, SessionPurpose::MemberBusiness, context())
                .await
        });
        let endpoint = pending.endpoint().clone();
        let server_coordinator = pending.coordinator();
        let server_context = original.clone();
        let inherited = kind == "incoming-token";
        let accepting_job = tokio::spawn(async move {
            let incoming = endpoint.accept().await.unwrap();
            if inherited {
                incoming
                    .with_caller_deadline(server_context.deadline)
                    .with_attempt_cancellation(server_context.cancellation.clone())
                    .await
                    .map_err(|error| ConnectError::Transport(error.to_string()))
            } else {
                let mut accepting = incoming.accept().unwrap();
                assert_eq!(accepting.alpn().await.unwrap(), MEMBER_ALPN);
                server_coordinator
                    .accept_accepting(accepting, SessionPurpose::MemberBusiness, server_context)
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(2), capture.blocked.notified())
            .await
            .unwrap();
        let observed = capture.observed.lock().unwrap().clone().unwrap();
        assert_eq!(observed.deadline, Some(original_deadline));
        assert!(observed.token.is_some());
        if kind != "accepting-deadline" {
            server_token.cancel();
        }
        let result = tokio::time::timeout(Duration::from_secs(3), accepting_job)
            .await
            .unwrap()
            .unwrap();
        match kind {
            "accepting-cancel" => assert!(matches!(result, Err(ConnectError::Cancelled))),
            "accepting-deadline" => assert!(matches!(result, Err(ConnectError::DeadlineExceeded))),
            _ => assert!(result.is_err()),
        }
        if kind != "accepting-deadline" {
            assert!(observed.token.unwrap().is_cancelled());
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(3), outgoing)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(!observed.original.is_open());
        assert!(healthy.close_reason().is_none());
        members::streams(&healthy, &healthy_peer).await;
        let sockets: Vec<_> = pending
            .addr()
            .ip_addrs()
            .chain(pending_client.addr().ip_addrs())
            .copied()
            .collect();
        tokio::join!(pending.close(), pending_client.close());
        let cleanup = closed_sockets(&sockets).await;
        println!(
            "NEGOTIATED_CONTEXT_RETIRE_RECEIPT {}",
            serde_json::json!({"kind":kind,"originalDeadlineMatched":true,
            "tokenMetadataPresent":true,"pendingOriginalClosed":true,"ordinaryHandle":false,"healthyMemberStreamMatched":true,"cleanup":cleanup})
        );
    }
    healthy.close(0u32.into(), b"owned healthy member ended");
    healthy_peer.close(0u32.into(), b"owned healthy member ended");
    drop((healthy, healthy_peer, healthy_ac, healthy_bc));
    tokio::join!(healthy_a.close(), healthy_b.close());
    tokio::join!(a.close(), b.close());
}
