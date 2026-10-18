//! Caller metadata and cancellation at the public connection/hook boundary.
use iroh::{
    Endpoint,
    endpoint::{
        AdmissionOutcome, ConnectOptions, ControlConnection, ControlExchange, ControlMessageFormat,
        EndpointAdmission, RestrictedEndpoint, presets,
    },
};
use n0_future::time::Instant;
use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
const ALPN: &[u8] = b"miopunch/member-attempt-test/1";
#[derive(Debug, Clone)]
struct UnreachableGate(Arc<tokio::sync::Barrier>);
impl EndpointAdmission for UnreachableGate {
    fn is_current(&self, _: &ControlConnection) -> bool {
        false
    }
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        let exchange = if connection.side().is_client() {
            connection.open_control_exchange().await?
        } else {
            connection.accept_control_exchange().await?
        };
        exchange
            .send_message(ControlMessageFormat::Fixed { length: 4 }, b"none")
            .await?;
        assert_eq!(
            exchange
                .recv_message(ControlMessageFormat::Fixed { length: 4 })
                .await?,
            b"none"
        );
        self.0.wait().await;
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "temporarily unreachable: no mutually permitted member data path",
        ))
    }
}
#[tokio::test]
async fn no_joint_member_data_path_is_reported_as_unreachable_without_a_handle() {
    let gate = UnreachableGate(Arc::new(tokio::sync::Barrier::new(2)));
    let a = endpoint(gate.clone()).await;
    let b = endpoint(gate).await;
    let (local, _) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(a.connect(b.addr(), ALPN), async {
            b.accept().await.unwrap().await
        })
    })
    .await
    .unwrap();
    let failure = local.err().expect("no normal handle");
    assert!(
        failure
            .to_string()
            .contains("temporarily unreachable: no mutually permitted member data path"),
        "public failure lost data-path reason: {failure}"
    );
    tokio::join!(a.close(), b.close());
}
#[derive(Debug, Clone)]
struct DeadlineGate {
    expected: Instant,
    observed: Arc<Mutex<Vec<Instant>>>,
}
impl EndpointAdmission for DeadlineGate {
    fn is_current(&self, _: &ControlConnection) -> bool {
        false
    }
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        assert_eq!(connection.caller_deadline(), Some(self.expected));
        let deadline = connection
            .caller_deadline()
            .expect("caller supplied one absolute deadline");
        self.observed.lock().unwrap().push(deadline);
        assert_eq!(deadline, self.expected);
        n0_future::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            std::future::pending::<()>(),
        )
        .await
        .map(|_| AdmissionOutcome::Pending)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "original caller deadline elapsed"))
    }
}
async fn endpoint(gate: impl EndpointAdmission + 'static) -> RestrictedEndpoint {
    Endpoint::builder(presets::Minimal)
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap()
        .clear_relay_transports()
        .alpns(vec![ALPN.to_vec()])
        .restricted_application(gate)
        .bind()
        .await
        .unwrap()
}
#[tokio::test]
async fn one_absolute_caller_deadline_reaches_both_protected_hooks_without_refresh() {
    let deadline = Instant::now() + Duration::from_millis(350);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let gate = DeadlineGate {
        expected: deadline,
        observed: observed.clone(),
    };
    let a = endpoint(gate.clone()).await;
    let b = endpoint(gate).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let started = std::time::Instant::now();
    let (local, remote) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            async {
                a.connect_with_opts(
                    b.addr(),
                    ALPN,
                    ConnectOptions::new().with_caller_deadline(deadline),
                )
                .await
                .unwrap()
                .await
            },
            async {
                b.accept()
                    .await
                    .unwrap()
                    .with_caller_deadline(deadline)
                    .await
            }
        )
    })
    .await
    .unwrap();
    assert!(
        local.is_err() && remote.is_err(),
        "no handle after the original caller deadline"
    );
    assert_eq!(*observed.lock().unwrap(), vec![deadline, deadline]);
    assert!(
        started.elapsed() < Duration::from_millis(320),
        "previous caller time is not renewed by TLS or the hooks"
    );
    tokio::join!(a.close(), b.close());
}

#[derive(Debug, Clone)]
struct HoldingGate {
    entered:
        Arc<Mutex<Option<tokio::sync::oneshot::Sender<iroh::endpoint::WeakControlConnection>>>>,
    stream: Arc<Mutex<Option<ControlExchange>>>,
    expects_cancellation: bool,
}
impl EndpointAdmission for HoldingGate {
    fn is_current(&self, _: &ControlConnection) -> bool {
        false
    }
    async fn admit(&self, connection: &ControlConnection) -> io::Result<AdmissionOutcome> {
        assert_eq!(
            connection.attempt_cancellation().is_some(),
            self.expects_cancellation
        );
        let exchange = if connection.side().is_client() {
            connection.open_control_exchange().await?
        } else {
            connection.accept_control_exchange().await?
        };
        exchange
            .send_message(ControlMessageFormat::Fixed { length: 4 }, b"hold")
            .await?;
        assert_eq!(
            exchange
                .recv_message(ControlMessageFormat::Fixed { length: 4 })
                .await?,
            b"hold"
        );
        *self.stream.lock().unwrap() = Some(exchange);
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(connection.weak_handle())
            .unwrap();
        connection.closed().await;
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "original attempt closed",
        ))
    }
}
#[tokio::test]
async fn dropping_a_pending_public_attempt_closes_original_m_even_with_a_held_private_stream() {
    let make = || {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            HoldingGate {
                entered: Arc::new(Mutex::new(Some(tx))),
                stream: Arc::new(Mutex::new(None)),
                expects_cancellation: false,
            },
            rx,
        )
    };
    let (left, left_entered) = make();
    let (right, right_entered) = make();
    let a = endpoint(left.clone()).await;
    let b = endpoint(right.clone()).await;
    let caller = a.clone();
    let address = b.addr();
    let attempt = tokio::spawn(async move { caller.connect(address, ALPN).await });
    let acceptor = b.clone();
    let peer = tokio::spawn(async move { acceptor.accept().await.unwrap().await });
    let (local, remote) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(left_entered, right_entered)
    })
    .await
    .unwrap();
    let (local, remote) = (local.unwrap(), remote.unwrap());
    assert!(local.is_open() && remote.is_open());
    attempt.abort();
    let _ = attempt.await;
    tokio::time::timeout(Duration::from_millis(300), async {
        while local.is_open() || remote.is_open() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drop closes the attempt even when a hook-held private stream still exists");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), peer)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    left.stream.lock().unwrap().take();
    right.stream.lock().unwrap().take();
    tokio::join!(a.close(), b.close());
}

#[tokio::test]
async fn explicit_attempt_cancellation_closes_only_original_m_before_returning_a_handle() {
    use tokio_util::sync::CancellationToken;
    let make = |expects_cancellation| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            HoldingGate {
                entered: Arc::new(Mutex::new(Some(tx))),
                stream: Arc::new(Mutex::new(None)),
                expects_cancellation,
            },
            rx,
        )
    };
    let (left, left_entered) = make(true);
    let (right, right_entered) = make(false);
    let a = endpoint(left.clone()).await;
    let b = endpoint(right.clone()).await;
    let cancellation = CancellationToken::new();
    let caller = a.clone();
    let address = b.addr();
    let token = cancellation.clone();
    let attempt = tokio::spawn(async move {
        caller
            .connect_with_opts(
                address,
                ALPN,
                ConnectOptions::new().with_attempt_cancellation(token),
            )
            .await
            .unwrap()
            .await
    });
    let acceptor = b.clone();
    let peer = tokio::spawn(async move { acceptor.accept().await.unwrap().await });
    let (local, remote) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(left_entered, right_entered)
    })
    .await
    .unwrap();
    let (local, remote) = (local.unwrap(), remote.unwrap());
    cancellation.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), attempt)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), peer)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        !local.is_open() && !remote.is_open(),
        "both original M ends are terminal"
    );
    assert!(
        !a.is_closed() && !b.is_closed(),
        "individual cancellation preserves the node endpoints"
    );
    left.stream.lock().unwrap().take();
    right.stream.lock().unwrap().take();
    tokio::join!(a.close(), b.close());
}
