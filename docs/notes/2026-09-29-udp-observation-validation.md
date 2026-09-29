# MIOPUNCH-19: UDP address observation without a data relay

2026-09-29. This validates the first UDP candidate-observation slice of
[MIOPUNCH-19](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6b266e50-15e1-4047-89f0-9f073334f592).
It uses iroh 1.2.0, a QAD QUIC service, and a real iroh UDP endpoint. The
fork delta and provenance are in [UPSTREAM.md](../../vendor/iroh/UPSTREAM.md).

## Behavior and result

`Endpoint::builder(presets::Minimal).observation_servers(map)` supplies QAD
servers without enabling any native data relay. The QAD client uses the same
noq endpoint and UDP transport as peer QUIC. Explicit observation servers are
QAD-only: no HTTPS or captive-portal probe is sent to them, and a QAD server
cannot become a preferred data relay. The portmapper remains independent of
whether this map is empty. The old empty-`RelayMap` early return no longer
prevents observation when a QAD map is supplied.

This validation API accepts an explicit QAD map only while native data relay
transports are disabled. Binding a mixed configuration fails explicitly rather
than silently changing data-relay selection. Supporting both maps together
requires separate probe and relay-selection reports in a later connection
slice; this does not change the no-relay behavior verified here.

`Endpoint::udp_observation()` reports `Disabled`, `Pending`, `Fresh`, or an
explicit failure reason. A fresh result contains the observed socket addresses,
observation time, 40-second validity deadline, and optional
`mapping_varies_by_dest`. It becomes fresh only after the same candidate has
entered `Endpoint::addr()`, the public address surface used for later direct
paths. It proves an address was observed by a QAD destination; it does not
claim a peer can reach that address. With only one observer, mapping variance
is `None` (unknown), not `false`.

QAD refresh runs every 20–26 seconds; each net report has a 5-second overall
probe window inside a 10-second caller timeout. A separate observation map
forces a new QAD check on each refresh instead of reusing an old QAD
connection. Failed refresh removes the old QAD candidate from `Endpoint::addr`
and reports `NoObservedAddress` or `TimedOut`. An explicit network-change hint
invalidates the result and refreshes immediately, including when a NAT mapping
changes without an interface-list change. An old in-flight report is discarded
after that invalidation. Consumers should read the current watcher result and
check `is_fresh()` before handing a candidate to another node. Even a copied
older `Fresh` result becomes invalid immediately when a newer report, a
failure, or a network change supersedes it; it also expires at its deadline
if an endpoint actor is delayed.

## Real socket and kernel NAT evidence

The integration seam is `tests/udp_observation.rs`. A standalone QAD server
speaks real QUIC. Its UDP source-address translation fixture records both the
real endpoint socket source and the translated source seen by the server; no
observation success is injected. All six tests passed on Rust 1.98.1:

- With native data relay disabled, the reported address equals the translated
  QAD source, `Endpoint.bound_sockets()` equals the fixture's incoming source,
  `Endpoint.addr()` contains the candidate, and relay status is empty.
- An empty observation map sends zero packets to the observation entrance.
- A changed mapping followed by `Endpoint.network_change()` replaces the old
  candidate. A failed periodic refresh after the observer disappears withdraws
  the old address in about 28 seconds; no relay connection appears.
- Two QAD destinations with different translated ports report
  `mapping_varies_by_dest: Some(true)`. This fixture models destination-related
  behavior; the kernel NAT run below uses one destination and reports `None`.
- Combining the new QAD map with a data-relay transport fails at bind, rather
  than silently changing relay selection.

The independent kernel path used two dedicated Docker bridge networks with a
client in `172.30.19.0/24` and a QAD service in `172.30.20.0/24`. Docker
published the QAD UDP service on host port 46762. Its kernel NAT changed the
client's source address and port across the bridges. The single captured run
showed:

| Surface | Address and packet |
| --- | --- |
| Client endpoint and client-side pcap | bound `0.0.0.0:33429`; QUIC UDP `172.30.19.2:33429 → 172.17.0.1:46762`, first datagram 1200 bytes |
| Observer-side pcap | QUIC UDP `172.30.20.1:44229 → 172.30.20.2:46761`, matching first datagram 1200 bytes and subsequent sequence |
| Endpoint public observation and address | `Fresh [172.30.20.1:44229]`; `Endpoint.addr()` contained both the local address and `172.30.20.1:44229`; relay status `[]` |

The paired captures are [client.pcap](../evidence/mio19-udp-observation/client.pcap)
(SHA-256 `5d302c004553c7d1bd2360680ad9e2cd370add4911d05a958cff32d857f83a94`)
and [observer.pcap](../evidence/mio19-udp-observation/observer.pcap)
(SHA-256 `d5be8d958c06b15b366e3b1e0f2c0cfba42654b59c4751daf8d1ff81bfbd5ed7`).
Only the dedicated experiment's QAD UDP port was captured. The synchronized
port and datagram sequence, plus the endpoint's public socket and candidate
values, tie the external address to the actual QUIC carrier socket. The
dedicated containers and networks were removed after capture.

## Reproduce

Run the fast public-interface tests with local UDP bind permission:

```sh
rtk cargo test --locked --test udp_observation -- --test-threads=1
```

The native CLI is `src/bin/udp_observation_probe.rs`. It generates a local
self-signed QAD certificate and supports `serve CERT_DER_PATH [BIND_ADDR]` and
`observe CERT_DER_PATH OBSERVER_HOST OBSERVER_QUIC_PORT [BIND_IP]`. A fresh
observation prints the bound socket, validity result, direct candidates, and
empty relay status. For the isolated Docker kernel path, build it and use two
otherwise unused bridge subnets and UDP ports. The following reproduces the
captured topology (subnets and ports can be changed if occupied):

```sh
rtk cargo build --locked --bin udp_observation_probe
rtk proxy mkdir -p /tmp/miopunch-19-nat
rtk proxy docker network create --driver bridge --subnet 172.30.19.0/24 miopunch19-nat-client
rtk proxy docker network create --driver bridge --subnet 172.30.20.0/24 miopunch19-nat-observer
rtk proxy docker run -d --name miopunch19-qad --network miopunch19-nat-observer --publish 46762:46761/udp --mount type=bind,src=/home/js/Git/miopunch/target/debug/udp_observation_probe,dst=/probe,readonly --mount type=bind,src=/tmp/miopunch-19-nat,dst=/evidence debian:bookworm /probe serve /evidence/cert.der 0.0.0.0:46761
rtk proxy docker logs miopunch19-qad
rtk proxy docker run --rm --network miopunch19-nat-client --add-host qad.miopunch.test:host-gateway --mount type=bind,src=/home/js/Git/miopunch/target/debug/udp_observation_probe,dst=/probe,readonly --mount type=bind,src=/tmp/miopunch-19-nat/cert.der,dst=/cert.der,readonly debian:bookworm /probe observe /cert.der qad.miopunch.test 46762
rtk proxy docker rm -f miopunch19-qad
rtk proxy docker network rm miopunch19-nat-client miopunch19-nat-observer
```

To repeat the packet comparison, keep a named client container running,
capture `udp port 46762` on its `eth0`, and capture `udp port 46761` on the
observer's `eth0` while invoking `observe`. The committed pcaps are the
output of this procedure. The container images used Debian bookworm and
tcpdump 4.99.3; Docker server was 29.6.2.

After `QAD_READY` appears in the log, run the client; the certificate file
must exist before Docker bind-mounts it. `cargo check --locked --workspace`,
`cargo fmt --all -- --check`, and the final full
`cargo test --locked --workspace -- --test-threads=1` passed. The final suite
reported 68 passed and one existing upstream ignored test across 11 suites.
An earlier full-suite run hit one unrelated, non-deterministic EOF-versus-reset
assertion in `src/tcp_transport.rs:646`; its single-test rerun and the final
full suite passed. The exact failure and follow-up are in the handoff for this
ticket.

This is one controlled kernel NAT and one QAD destination, plus independent
destination-related behavior in the integration fixture. It does not establish
peer reachability, a successful UDP hole punch, or a result for other NATs or
operators. Those remain dependent tickets.
