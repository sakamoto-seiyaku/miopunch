//! Temporary diagnostic: per relay, separate QUIC handshake time from the
//! time until the relay reports an observed address, on a plain noq endpoint.

use std::{net::ToSocketAddrs, sync::Arc, time::Duration};

use iroh_relay::quic::QuicClient;
use tokio::time::{Instant, timeout};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let endpoint = noq::Endpoint::client("0.0.0.0:0".parse()?)?;
    let client = QuicClient::new(endpoint.clone(), tls);
    // Pairs of (address host, SNI); defaults to each relay with its own name.
    let args: Vec<String> = std::env::args().skip(2).collect();
    let pairs: Vec<(String, String)> = if args.is_empty() {
        ["aps1-1.relay.n0.iroh.link.", "euc1-1.relay.n0.iroh.link.", "use1-1.relay.n0.iroh.link.", "usw1-1.relay.n0.iroh.link."]
            .iter().map(|h| (h.to_string(), h.to_string())).collect()
    } else {
        args.iter().map(|a| { let (ip, sni) = a.split_once('=').expect("ADDRHOST=SNI"); (ip.to_string(), sni.to_string()) }).collect()
    };
    let rounds: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(2);
    for round in 0..rounds {
        for (target, sni) in &pairs {
            let host = sni.as_str();
            let addr = (target.as_str(), 7842).to_socket_addrs()?.find(|a| a.is_ipv4()).ok_or("no v4")?;
            let started = Instant::now();
            let conn = match timeout(Duration::from_secs(10), client.create_conn(addr, host)).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(error)) => {
                    println!("round={round} {host} {addr} handshake-error {}ms {error}", started.elapsed().as_millis());
                    continue;
                }
                Err(_) => {
                    println!("round={round} {host} {addr} handshake-timeout 10s");
                    continue;
                }
            };
            let handshake = started.elapsed().as_millis();
            let mut watcher = conn.observed_external_addr();
            let observed = timeout(Duration::from_secs(10), async {
                use n0_future::StreamExt as _;
                watcher.next().await
            })
            .await;
            let total = started.elapsed().as_millis();
            let alpn = conn.handshake_data().and_then(|d| d.downcast::<noq::crypto::rustls::HandshakeData>().ok()).and_then(|d| d.protocol.clone()).map(|p| String::from_utf8_lossy(&p).to_string());
            match observed {
                Ok(Some(observed)) => println!("round={round} {host} {addr} handshake={handshake}ms observed={observed} at={total}ms alpn={alpn:?}"),
                Ok(None) => println!("round={round} {host} {addr} handshake={handshake}ms observed=stream-ended at={total}ms alpn={alpn:?}"),
                Err(_) => println!("round={round} {host} {addr} handshake={handshake}ms observed=TIMEOUT(10s) alpn={alpn:?} close={:?}", conn.close_reason()),
            }
            conn.close(0u32.into(), b"probe");
        }
    }
    endpoint.wait_idle().await;
    let _ = Arc::new(());
    Ok(())
}
