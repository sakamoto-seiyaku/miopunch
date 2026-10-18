//! Ordinary address-book failures stay local; resolving is an explicit choice.
use std::{
    net::{Ipv4Addr, Ipv6Addr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::{
    EndpointAddr, EndpointId,
    address_lookup::{AddressLookup, Error, Item, memory::MemoryLookup},
    endpoint::{AfterHandshakeOutcome, BeforeConnectOutcome, Connection, EndpointHooks},
    protocol::{AcceptError, ProtocolHandler},
};
use n0_future::boxed::BoxStream;
use p2panda_net::{
    AddressBook, Endpoint,
    addrs::NodeInfo,
    discovery::{DiscoveryBackoff, DiscoveryBackoffConfig},
    iroh_endpoint::{EndpointError, IrohConfig},
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

#[derive(Debug, Clone)]
struct CountLookup {
    calls: Arc<AtomicUsize>,
    inner: MemoryLookup,
}

impl AddressLookup for CountLookup {
    fn resolve(&self, id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve(id)
    }
}

fn config() -> IrohConfig {
    IrohConfig {
        bind_ip_v4: Ipv4Addr::LOCALHOST,
        bind_port_v4: 0,
        bind_ip_v6: Ipv6Addr::LOCALHOST,
        bind_port_v6: 0,
    }
}

#[tokio::test]
async fn ordinary_missing_transport_never_starts_configured_lookup() {
    for (known_node, with_config) in [(false, false), (true, false), (false, true), (true, true)] {
        let book = AddressBook::builder().spawn().await.unwrap();
        let target = p2panda_core::SigningKey::generate().verifying_key();
        if known_node {
            book.insert_node_info(NodeInfo::new(target)).await.unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let lookup = CountLookup {
            calls: calls.clone(),
            inner: MemoryLookup::default(),
        };
        let endpoint = Endpoint::builder(book)
            .config(config())
            .endpoint_builder_hook(move |builder| {
                builder
                    .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                    .address_lookup(lookup.clone())
            })
            .spawn()
            .await
            .unwrap();
        let inner = endpoint.endpoint().await.unwrap();
        let started = Instant::now();
        let attempt = async {
            if with_config {
                endpoint
                    .connect_with_config(
                        target,
                        b"resolution-contract/1",
                        iroh::endpoint::QuicTransportConfig::default(),
                    )
                    .await
            } else {
                endpoint.connect(target, b"resolution-contract/1").await
            }
        };
        let result = tokio::time::timeout(Duration::from_millis(750), attempt).await;
        let elapsed = started.elapsed();
        inner.close().await;
        drop((endpoint, inner));
        let expected =
            format!("address book does not have any iroh address info for node id {target}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "known_node={known_node}");
        assert!(
            matches!(&result, Ok(Err(EndpointError::Connect(err))) if err.to_string() == expected),
            "known_node={known_node}: {result:?}"
        );
        assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
    }
}

const PROTOCOL: &[u8] = b"resolution-contract/1";

#[derive(Debug)]
struct Echo;

impl ProtocolHandler for Echo {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        assert_eq!(
            recv.read_to_end(4).await.map_err(std::io::Error::other)?,
            b"PING"
        );
        send.write_all(b"PONG")
            .await
            .map_err(std::io::Error::other)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn resolving_uses_the_configured_lookup_and_exchanges_real_streams() {
    let server_book = AddressBook::builder().spawn().await.unwrap();
    let server = Endpoint::builder(server_book)
        .config(config())
        .endpoint_builder_hook(|builder| {
            builder.portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        })
        .spawn()
        .await
        .unwrap();
    server.accept(PROTOCOL, Echo).await.unwrap();
    let server_raw = server.endpoint().await.unwrap();
    let address = server_raw.addr();

    for with_config in [false, true] {
        // A fresh endpoint prevents the preceding case's cached addresses from
        // satisfying this attempt without the configured lookup.
        let book = AddressBook::builder().spawn().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let lookup = CountLookup {
            calls: calls.clone(),
            inner: MemoryLookup::from_endpoint_info([address.clone()]),
        };
        let client = Endpoint::builder(book.clone())
            .config(config())
            .endpoint_builder_hook(move |builder| {
                builder
                    .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                    .address_lookup(lookup.clone())
            })
            .spawn()
            .await
            .unwrap();
        let client_raw = client.endpoint().await.unwrap();
        let ordinary = client.connect(server.node_id(), PROTOCOL).await;
        assert!(ordinary.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(book.node_info(server.node_id()).await.unwrap().is_none());

        let attempt = async {
            if with_config {
                client
                    .connect_resolving_with_config(
                        server.node_id(),
                        PROTOCOL,
                        iroh::endpoint::QuicTransportConfig::default(),
                    )
                    .await
            } else {
                client.connect_resolving(server.node_id(), PROTOCOL).await
            }
        };
        let connection = tokio::time::timeout(Duration::from_secs(5), attempt)
            .await
            .unwrap()
            .unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(b"PING").await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(4).await.unwrap(), b"PONG");
        let lookups = calls.load(Ordering::SeqCst);
        connection.close(0u32.into(), b"resolving stream complete");
        client_raw.close().await;
        drop((connection, client, client_raw));
        assert!(lookups > 0);
        println!(
            "RESOLVING_STREAM with_config={with_config} lookup_calls={lookups} ping_pong=true closed=true"
        );
    }
    server_raw.close().await;
    drop((server, server_raw));
}

#[tokio::test]
async fn address_book_store_failure_stays_local_for_both_entry_points() {
    for resolving in [false, true] {
        let store = p2panda_store::SqliteStoreBuilder::new()
            .build()
            .await
            .unwrap();
        let book = AddressBook::builder()
            .store(store.clone())
            .spawn()
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let lookup = CountLookup {
            calls: calls.clone(),
            inner: MemoryLookup::default(),
        };
        let endpoint = Endpoint::builder(book)
            .config(config())
            .endpoint_builder_hook(move |builder| {
                builder
                    .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                    .clear_address_lookup()
                    .address_lookup(lookup.clone())
            })
            .spawn()
            .await
            .unwrap();
        let raw = endpoint.endpoint().await.unwrap();
        store.pool().close().await;
        let target = p2panda_core::SigningKey::generate().verifying_key();
        let attempt = async {
            if resolving {
                endpoint.connect_resolving(target, PROTOCOL).await
            } else {
                endpoint.connect(target, PROTOCOL).await
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(1), attempt).await;
        raw.close().await;
        drop((endpoint, raw));
        let err = result.unwrap().unwrap_err();
        assert!(format!("{err:?}").contains("AddressBook("), "{err:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        println!(
            "STORE_FAILURE resolving={resolving} lookup_calls=0 classified=address_book closed=true"
        );
    }
}

#[derive(Debug, Clone)]
struct FailingLookup(Arc<AtomicUsize>);

impl AddressLookup for FailingLookup {
    fn resolve(&self, _: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Some(Box::pin(futures_util::stream::iter([Err(
            Error::from_err(
                "owned-failing-lookup",
                std::io::Error::other("owned lookup failure"),
            ),
        )])))
    }
}

#[tokio::test]
async fn resolving_lookup_failure_is_not_a_local_admission_rejection() {
    let book = AddressBook::builder().spawn().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup = FailingLookup(calls.clone());
    let endpoint = Endpoint::builder(book)
        .config(config())
        .endpoint_builder_hook(move |builder| {
            builder
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .clear_address_lookup()
                .address_lookup(lookup.clone())
        })
        .spawn()
        .await
        .unwrap();
    let raw = endpoint.endpoint().await.unwrap();
    let target = p2panda_core::SigningKey::generate().verifying_key();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        endpoint.connect_resolving(target, PROTOCOL),
    )
    .await;
    raw.close().await;
    drop((endpoint, raw));
    let err = result.unwrap().unwrap_err();
    let detail = format!("{err:?}");
    assert!(
        detail.contains("No addressing information available"),
        "{detail}"
    );
    assert!(detail.contains("owned lookup failure"), "{detail}");
    assert!(
        !detail.contains("Connection was rejected locally"),
        "{detail}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    println!("LOOKUP_FAILURE calls=1 classified=no_address local_rejection=false closed=true");
}

#[derive(Debug, Clone)]
struct RejectLocally {
    before: Arc<AtomicBool>,
    after: Arc<AtomicBool>,
    handshakes: Arc<AtomicUsize>,
}

impl EndpointHooks for RejectLocally {
    async fn before_connect(&self, _: &EndpointAddr, _: &[u8]) -> BeforeConnectOutcome {
        if self.before.load(Ordering::SeqCst) {
            BeforeConnectOutcome::Reject
        } else {
            BeforeConnectOutcome::Accept
        }
    }

    async fn after_handshake(&self, _: &Connection) -> AfterHandshakeOutcome {
        self.handshakes.fetch_add(1, Ordering::SeqCst);
        if self.after.load(Ordering::SeqCst) {
            AfterHandshakeOutcome::Reject {
                error_code: 1u32.into(),
                reason: b"owned local rejection".to_vec(),
            }
        } else {
            AfterHandshakeOutcome::Accept
        }
    }
}

#[derive(Debug)]
struct WaitForClose;
impl ProtocolHandler for WaitForClose {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        connection.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn local_rejection_preserves_health_while_real_handshake_failure_is_reported() {
    // The evidence runner creates this address solely inside a network-none
    // Docker namespace. It makes p2panda's existing global-reachability guard
    // admit failure reports, so a missing report cannot be hidden by loopback.
    let mut bind = config();
    let global = std::env::var("MIOPUNCH_TEST_PUBLIC_ALIAS").is_ok();
    if global {
        assert_eq!(
            std::env::var("MIOPUNCH_TEST_PUBLIC_ALIAS").unwrap(),
            "11.0.0.1"
        );
        bind.bind_ip_v4 = Ipv4Addr::new(11, 0, 0, 1);
    }
    let server = Endpoint::builder(AddressBook::builder().spawn().await.unwrap())
        .config(bind.clone())
        .endpoint_builder_hook(|builder| {
            builder.portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        })
        .spawn()
        .await
        .unwrap();
    server.accept(PROTOCOL, WaitForClose).await.unwrap();
    let server_raw = server.endpoint().await.unwrap();
    let book = AddressBook::builder().spawn().await.unwrap();
    book.insert_node_info(NodeInfo::from(server_raw.addr()))
        .await
        .unwrap();
    let hook = RejectLocally {
        before: Arc::new(AtomicBool::new(true)),
        after: Arc::new(AtomicBool::new(false)),
        handshakes: Arc::new(AtomicUsize::new(0)),
    };
    let configured = hook.clone();
    let client = Endpoint::builder(book.clone())
        .config(bind)
        .endpoint_builder_hook(move |builder| {
            builder
                .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                .hooks(configured.clone())
        })
        .spawn()
        .await
        .unwrap();
    let client_raw = client.endpoint().await.unwrap();
    if global {
        assert!(
            client_raw
                .addr()
                .ip_addrs()
                .any(|addr| addr.ip() == Ipv4Addr::new(11, 0, 0, 1))
        );
    }
    let before = book
        .node_info(server.node_id())
        .await
        .unwrap()
        .unwrap()
        .metrics;
    let mut errors = Vec::new();
    for after_handshake in [false, true] {
        hook.before.store(!after_handshake, Ordering::SeqCst);
        hook.after.store(after_handshake, Ordering::SeqCst);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            client.connect(server.node_id(), PROTOCOL),
        )
        .await
        .unwrap();
        let err = result.unwrap_err();
        errors.push((format!("{err:?}"), err.to_string()));
    }
    // The actor queues reports before replying to connect. This following
    // endpoint query orders the address-book read after any queued report.
    client.endpoint().await.unwrap();
    let after_rejections = book
        .node_info(server.node_id())
        .await
        .unwrap()
        .unwrap()
        .metrics;
    hook.after.store(false, Ordering::SeqCst);
    let failure = tokio::time::timeout(
        Duration::from_secs(5),
        client.connect(server.node_id(), b"unsupported-protocol/1"),
    )
    .await
    .unwrap();
    let network_error = format!("{:?}", failure.unwrap_err());
    client.endpoint().await.unwrap();
    let after_network = book
        .node_info(server.node_id())
        .await
        .unwrap()
        .unwrap()
        .metrics;
    tokio::join!(client_raw.close(), server_raw.close());
    drop((client, server, client_raw, server_raw));
    assert!(errors[0].0.contains("ConnectWithOpts("), "{errors:?}");
    assert!(errors[1].0.contains("Connecting("), "{errors:?}");
    assert!(
        errors
            .iter()
            .all(|(_, message)| message == "Connection was rejected locally"),
        "{errors:?}"
    );
    assert_eq!(hook.handshakes.load(Ordering::SeqCst), 1);
    assert_eq!(after_rejections, before);
    assert!(
        !network_error.contains("Connection was rejected locally"),
        "{network_error}"
    );
    assert!(network_error.contains("Connecting("), "{network_error}");
    if global {
        assert!(
            after_network.is_stale(),
            "a genuine failure must be recorded"
        );
    } else {
        assert_eq!(
            after_network, before,
            "the existing loopback reporting guard remains"
        );
    }
    println!(
        "FAILURE_REPORTS global_guard_admitted={global} pre_and_post_local_rejection=true health_preserved=true genuine_handshake_failure_stale={} closed=true",
        after_network.is_stale()
    );
}

#[test]
fn discovery_backoff_rejects_unrepresentable_increment_bounds() {
    let ms = Duration::from_millis;
    for (minimum, maximum) in [
        (Duration::ZERO, ms(2)),
        (ms(2), ms(2)),
        (ms(3), ms(2)),
        (Duration::from_nanos(1), Duration::from_nanos(2)),
        (Duration::from_micros(1001), Duration::from_micros(1999)),
        (ms(1), Duration::from_micros(2001)),
        (ms(1), Duration::MAX),
    ] {
        assert!(
            DiscoveryBackoffConfig::bounded(Duration::ZERO, minimum, maximum, ms(20)).is_err(),
            "accepted {minimum:?}..{maximum:?}"
        );
    }
    assert!(DiscoveryBackoffConfig::bounded(ms(3), ms(1), ms(2), ms(2)).is_err());
}

#[test]
fn discovery_backoff_clamps_immediately_without_duration_overflow() {
    let initial = Duration::from_micros(500);
    let ceiling = Duration::from_micros(2500);
    let cfg = DiscoveryBackoffConfig::bounded(
        initial,
        Duration::from_millis(1),
        Duration::from_millis(2),
        ceiling,
    )
    .unwrap();
    let mut backoff = DiscoveryBackoff::new(cfg, ChaCha20Rng::from_seed([1; 32]));
    assert_eq!(backoff.duration(), initial);
    backoff.increment();
    assert_eq!(backoff.duration(), Duration::from_micros(1500));
    backoff.increment();
    assert_eq!(backoff.duration(), ceiling);
    for _ in 0..100 {
        backoff.increment();
        assert_eq!(backoff.duration(), ceiling);
    }

    let cfg = DiscoveryBackoffConfig::bounded(
        Duration::MAX - Duration::from_micros(500),
        Duration::from_millis(1),
        Duration::from_millis(2),
        Duration::MAX,
    )
    .unwrap();
    let mut backoff = DiscoveryBackoff::new(cfg, ChaCha20Rng::from_seed([2; 32]));
    backoff.increment();
    assert_eq!(backoff.duration(), Duration::MAX);

    let cfg = DiscoveryBackoffConfig::bounded(
        Duration::ZERO,
        Duration::from_millis(u64::MAX - 1),
        Duration::from_millis(u64::MAX),
        Duration::MAX,
    )
    .unwrap();
    let mut backoff = DiscoveryBackoff::new(cfg, ChaCha20Rng::from_seed([3; 32]));
    backoff.increment();
    assert_eq!(backoff.duration(), Duration::from_millis(u64::MAX - 1));
}

#[test]
fn default_backoff_retains_random_increments_and_its_ceiling() {
    let mut backoff = DiscoveryBackoff::new(
        DiscoveryBackoffConfig::default(),
        ChaCha20Rng::from_seed([1; 32]),
    );
    let mut increments = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let before = backoff.duration();
        backoff.increment();
        let after = backoff.duration();
        assert!(after >= before);
        assert!(after <= Duration::from_secs(30));
        if before <= Duration::from_secs(25) {
            let increment = after - before;
            assert!(increment >= Duration::from_secs(1));
            assert!(increment < Duration::from_secs(5));
            increments.insert(increment);
        }
    }
    assert_eq!(backoff.duration(), Duration::from_secs(30));
    assert!(increments.len() > 1);
}
