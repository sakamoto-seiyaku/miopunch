//! The existing process-wide diagnostic hook is exercised in fresh processes.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use noq_proto::packet_witness::{self as witness, Record, Value};

#[derive(Default, Debug)]
struct Observations {
    callbacks: usize,
    contexts: usize,
    weak_checks: usize,
    copied_bytes: usize,
    digest_fields: usize,
    stages: BTreeMap<&'static str, usize>,
    carrier_by_connection: BTreeMap<usize, usize>,
}

fn install(
    mode: &str,
    observations: Arc<Mutex<Observations>>,
    weak: Arc<Mutex<Option<noq::WeakConnectionHandle>>>,
) {
    let mode = mode.to_owned();
    witness::install(move |record: Record| {
        // Metadata only: never wait for another task or acquire the connection lock.
        let mut seen = observations
            .try_lock()
            .expect("observer metadata is never held across I/O");
        seen.callbacks += 1;
        seen.contexts += usize::from(record.connection_stable_id.is_some());
        *seen.stages.entry(record.stage).or_default() += 1;
        if record.stage == "before-actual-carrier-call" {
            if let Some(stable) = record.connection_stable_id {
                *seen.carrier_by_connection.entry(stable).or_default() += 1;
            }
        }
        for (name, value) in &record.fields {
            match value {
                Value::Bytes(bytes) => seen.copied_bytes += bytes.len(),
                Value::Text(digest) if name.ends_with("sha256") => {
                    assert_eq!(digest.len(), 64);
                    seen.digest_fields += 1;
                }
                _ => {}
            }
        }
        if let Some(weak) = weak.try_lock().expect("weak metadata never waits").as_ref() {
            let _ = weak.is_open();
            seen.weak_checks += 1;
        }
        drop(seen);
        if mode == "recursive" {
            witness::emit("recursive-observer", vec![]);
        } else if mode == "panic" {
            panic!("owned diagnostic callback failure");
        }
    })
    .unwrap();
}

async fn ordinary_stream(
    weak: &Mutex<Option<noq::WeakConnectionHandle>>,
    seen: &Mutex<Observations>,
) -> (u64, usize) {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let der = CertificateDer::from(cert.cert);
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let config = noq::ServerConfig::with_single_cert(vec![der.clone()], key.into()).unwrap();
    let server_endpoint = noq::Endpoint::server(config, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(der).unwrap();
    let mut client_endpoint = noq::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    client_endpoint.set_default_client_config(
        noq::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap(),
    );
    let client_addr = client_endpoint.local_addr().unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();
    let (client, server) = tokio::time::timeout(Duration::from_secs(3), async {
        let (c, s) = tokio::join!(
            client_endpoint.connect(server_addr, "localhost").unwrap(),
            async { server_endpoint.accept().await.unwrap().await }
        );
        (c.unwrap(), s.unwrap())
    })
    .await
    .unwrap();
    *weak.lock().unwrap() = Some(client.weak_handle());
    let payload = vec![0x5a; 32_768];
    let mut send = client.open_uni().await.unwrap();
    send.write_all(&payload).await.unwrap();
    send.finish().unwrap();
    let mut recv = server.accept_uni().await.unwrap();
    assert_eq!(recv.read_to_end(65_536).await.unwrap(), payload);
    assert_eq!(send.stopped().await.unwrap(), None);
    assert_eq!(client.path_filter_stats(), noq::PathFilterStats::default());

    // A diagnostic can observe encoding; that cannot authorize the actual send.
    client.restrict_network_paths(|_| false);
    let before = *seen
        .lock()
        .unwrap()
        .carrier_by_connection
        .get(&client.stable_id())
        .unwrap_or(&0);
    let mut forbidden = client.open_uni().await.unwrap();
    forbidden
        .write_all(b"diagnostic-does-not-authorize")
        .await
        .unwrap();
    forbidden.finish().unwrap();
    let delivered = tokio::time::timeout(Duration::from_millis(150), server.accept_uni())
        .await
        .is_ok();
    let denied = client.path_filter_stats().denied_outbound;
    let after = *seen
        .lock()
        .unwrap()
        .carrier_by_connection
        .get(&client.stable_id())
        .unwrap_or(&0);

    client.close(0u32.into(), b"ended");
    assert!(!weak.lock().unwrap().as_ref().unwrap().is_open());
    server.close(0u32.into(), b"ended");
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client_endpoint.wait_idle(), server_endpoint.wait_idle());
    })
    .await
    .unwrap();
    drop(send);
    drop(recv);
    drop(forbidden);
    drop(client);
    drop(server);
    drop(client_endpoint);
    drop(server_endpoint);
    for addr in [client_addr, server_addr] {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match std::net::UdpSocket::bind(addr) {
                    Ok(socket) => {
                        drop(socket);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("owned diagnostic UDP rebind: {error}"),
                }
            }
        })
        .await
        .unwrap();
    }
    assert!(!delivered, "witness cannot deliver a forbidden stream");
    assert!(denied > 0, "the real packet guard ran");
    assert_eq!(
        before, after,
        "encoding records do not bypass the final packet gate"
    );
    (denied, payload.len())
}

#[tokio::test]
#[ignore = "launched in a fresh process by diagnostic_opt_in_contract"]
async fn diagnostic_process() {
    let mode = std::env::var("MIOPUNCH_DIAGNOSTIC_CASE").unwrap();
    let seen = Arc::new(Mutex::new(Observations::default()));
    let weak = Arc::new(Mutex::new(None));
    assert!(!witness::enabled());
    if mode != "off" {
        install(&mode, seen.clone(), weak.clone());
        assert!(witness::enabled());
    }
    match mode.as_str() {
        "off" | "on" | "recursive" | "panic" => {}
        "finished" => {
            assert_eq!(witness::finish().records, 0);
        }
        "record-cap" => {
            while witness::enabled() {
                witness::emit("record-cap", vec![]);
            }
        }
        "byte-cap" => {
            while witness::enabled() {
                witness::emit("byte-cap", vec![("bytes", Value::Bytes(vec![0; 65_536]))]);
            }
        }
        "field-cap" => witness::emit("field-cap", vec![("value", Value::Bool(false)); 33]),
        "value-cap" => witness::emit("value-cap", vec![("bytes", Value::Bytes(vec![0; 65_537]))]),
        "text-cap" => witness::emit("text-cap", vec![("text", Value::Text("x".repeat(262_145)))]),
        _ => panic!("unknown diagnostic case"),
    }
    if !matches!(mode.as_str(), "off" | "on" | "recursive" | "panic") {
        assert!(!witness::enabled());
    }
    let (denied, bytes) = ordinary_stream(&weak, &seen).await;
    // All producers and sockets have ended before finish; it is not a join barrier.
    let status = witness::finish();
    assert!(!witness::enabled());
    let callbacks = seen.lock().unwrap().callbacks;
    witness::emit("after-finish", vec![]);
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(seen.callbacks, callbacks);
    assert!(status.records <= 16_384 && status.bytes <= 16 * 1024 * 1024);
    match mode.as_str() {
        "off" | "finished" => {
            assert_eq!(status.records, 0);
            assert_eq!(status.bytes, 0);
            assert_eq!(status.lost, 0);
            assert_eq!(seen.callbacks, 0);
        }
        "on" => {
            assert_eq!(status.lost, 0);
            assert_eq!(status.records, seen.callbacks);
            assert!(
                seen.contexts > 0
                    && seen.weak_checks > 0
                    && seen.copied_bytes > 0
                    && seen.digest_fields > 0
            );
            for stage in [
                "builder-open",
                "actual-payload-appended",
                "actual-packet-encrypted",
                "before-actual-carrier-call",
                "actual-sender-returned-ok",
            ] {
                assert!(
                    seen.stages.get(stage).copied().unwrap_or(0) > 0,
                    "missing actual stage {stage}"
                );
            }
        }
        "recursive" | "panic" => {
            assert_eq!(status.lost, 1);
            assert_eq!(status.records, 1);
            assert_eq!(seen.callbacks, 1);
            assert_eq!(
                seen.contexts, 1,
                "callback failure occurred in an actual drive poll"
            );
        }
        "record-cap" => {
            assert_eq!(status.lost, 1);
            assert_eq!(status.records, 16_384);
            assert_eq!(seen.callbacks, 16_384);
        }
        "byte-cap" => {
            assert_eq!(status.lost, 1);
            assert!(status.records > 0);
            assert!(status.bytes + 65_541 > 16 * 1024 * 1024);
        }
        "field-cap" | "value-cap" | "text-cap" => {
            assert_eq!(status.lost, 1);
            assert_eq!(status.records, 0);
            assert_eq!(seen.callbacks, 0);
        }
        _ => unreachable!(),
    }
    if mode != "off" {
        assert_eq!(
            witness::install(|_| {}).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }
    println!(
        "DIAGNOSTIC_WORK {}",
        serde_json::json!({"mode":mode,"records":status.records,"reservedFieldBytes":status.bytes,"lost":status.lost,"callbacks":seen.callbacks,"connectionContexts":seen.contexts,"weakAtomicChecks":seen.weak_checks,"copiedBytesInObservedFields":seen.copied_bytes,"observedSha256Fields":seen.digest_fields,"stages":seen.stages,"ordinaryStreamBytes":bytes,"deniedActualOutbound":denied,"forbiddenStreamDelivered":false,"ownedUdpPortsRebound":true,"observerClosed":true,"scope":"record/field/callback counts, not total heap usage, CPU cost, timing equivalence or physical-send proof"})
    );
}

#[test]
fn diagnostic_opt_in_contract() {
    for mode in [
        "off",
        "on",
        "finished",
        "recursive",
        "panic",
        "record-cap",
        "byte-cap",
        "field-cap",
        "value-cap",
        "text-cap",
    ] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "diagnostic_process", "--nocapture"])
            .env("MIOPUNCH_DIAGNOSTIC_CASE", mode)
            .output()
            .unwrap();
        let stdout = String::from_utf8(result.stdout).unwrap();
        assert!(
            result.status.success(),
            "diagnostic case {mode}: {stdout}\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(stdout.matches("DIAGNOSTIC_WORK ").count(), 1);
        print!("{stdout}");
    }
}
