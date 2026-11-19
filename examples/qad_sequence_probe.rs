//! Temporary diagnostic: time the production QAD mapping sequence against the
//! default relays, in full and in pairs, to find which observer stalls.

use std::time::Instant;

use iroh::{Endpoint, RelayMap, endpoint::presets};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let relays = iroh::defaults::prod::default_relay_map().relays::<Vec<_>>();
    for relay in &relays {
        println!("RELAY {} quic={:?}", relay.url, relay.quic);
    }
    let endpoint = Endpoint::builder(presets::N0).bind().await?;
    println!("BOUND {:?}", endpoint.bound_sockets());
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    {
        use iroh::Watcher as _;
        println!("UDP_OBSERVATION {:?}", endpoint.udp_observation().get());
        println!("ADDR {:?}", endpoint.addr());
    }
    let rounds: usize = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    let mut sets = vec![relays.clone()];
    for (index, first) in relays.iter().enumerate() {
        for second in &relays[index + 1..] {
            sets.push(vec![first.clone(), second.clone()]);
        }
    }
    for round in 0..rounds {
        for set in &sets {
            let map = RelayMap::from_iter(set.iter().cloned());
            let names: Vec<_> = set
                .iter()
                .map(|relay| relay.url.host_str().unwrap_or("?").split('.').next().unwrap_or("?").to_string())
                .collect();
            let started = Instant::now();
            let result = endpoint.observe_udp_mapping_sequence(&map).await;
            let elapsed = started.elapsed().as_millis();
            match result {
                Ok(sequence) => {
                    let samples: Vec<_> = sequence
                        .samples
                        .iter()
                        .map(|sample| format!("{}->{}", sample.destination, sample.mapped))
                        .collect();
                    println!("round={round} set={names:?} ok {elapsed}ms {samples:?}");
                }
                Err(error) => println!("round={round} set={names:?} ERR {elapsed}ms {error}"),
            }
        }
    }
    endpoint.close().await;
    Ok(())
}
