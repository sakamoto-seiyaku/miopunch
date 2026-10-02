# iroh fork for UDP address observation

This directory starts from the published `iroh` crate version `1.2.0` in the
local crates.io registry. `Cargo.toml` is the published, normalized manifest;
`Cargo.toml.orig` preserves the upstream workspace manifest. The upstream
license files remain in this directory.

MIOPUNCH-19 changes only `src/endpoint.rs` and `src/socket.rs`:

- `Builder::observation_servers` configures QAD servers independently of data
  relay transports. Explicit observation servers run QAD without HTTPS or
  captive-portal probes and cannot be selected as a data relay. This validation
  path requires data relay transports disabled; a mixed configuration fails at
  bind rather than silently altering relay selection.
- Empty observation configuration still permits the independent portmapper.
- `Endpoint::udp_observation` exposes observed UDP candidates, a 40-second
  validity deadline, destination-dependent mapping information, and bounded
  failure states. A fresh state is published after `Endpoint::addr` is updated.
- Periodic full QAD reports replace stale candidates; an explicit
  `Endpoint::network_change` refreshes even if the interface list is unchanged.
  Generation checks discard results from an older in-flight QAD run.

The fork is selected for every workspace dependency with the root
`[patch.crates-io]` entry. See
`docs/notes/2026-09-29-udp-observation-validation.md` for the real UDP and
kernel-NAT evidence and the scope of this validation.

## MIOPUNCH-21 data-path validation delta

This ticket adds `control_only_alpns` and explicit `open_data_path`,
`open_control_path`, and `commit_data_path` calls. The per-peer actor suppresses
automatic candidate disclosure for restricted connections, keeps custom paths
off committed data connections, and closes their original custom path after an
established UDP path exists. Restricted dials with one explicit custom route
send their Initial to that route, even when another connection selected IP;
explicit path-open failures do not enter the peer-wide retry queue. A small `noq` packet gate supplies the required
send-before-`Established` and receive-before-QUIC admission boundary; merely
changing iroh's selected path cannot provide that boundary. An endpoint with
control-only ALPNs also disables TLS early data and refuses 0-RTT/0.5-RTT
handles. `Accepting::into_0rtt` now returns `Result` in this fork.

This is a local API change, not a QUIC wire change. The acceptance experiment
and remaining boundaries are in
`docs/notes/2026-09-29-mqtt-data-path-validation.md`.
The `miopunch-test-hooks` feature adds a test-only forced custom-path reopen
to exercise the receive gate against a fresh authenticated remote attempt;
the normal dependency does not enable this feature.

## MIOPUNCH-27 dual-NAT validation delta

`Endpoint::prime_udp_path` sends one 11-byte non-QUIC packet from the same
UDP socket that QAD observed and QUIC uses. The per-peer actor permits this
only while the supplied authenticated connection is live and restricted to
control. This lets the accepting side open an address-dependent NAT filter
for its peer before the dialing side validates an IP path; the accepting side
cannot initiate noq's new QUIC path on that connection. A paired no-prime
run demonstrates the filter blocking that path. The call does not establish
a data path or authorize business traffic. `open_data_path` reports unavailable
path identifiers as `WouldBlock`, allowing the caller to retry that exact
connection and address within a bounded window. The optional
`miopunch-test-hooks` feature can inject one such transient result for a
deterministic branch check while subsequent QUIC/UDP behavior remains real.
That injected error is distinct from a naturally exhausted CID pool. No QUIC
wire format or generic hole-punch policy is changed. The real dual-NAT
reproduction and remaining NAT classes are recorded in
`docs/notes/2026-09-29-udp-nat-validation.md`.

## MIOPUNCH-56 NAT3 promotion race delta

When both endpoints upgrade one authenticated QUIC connection from the
restricted MQTT path to a validated UDP path, the first endpoint can abandon
the custom path before the second calls `commit_data_path`. The second
endpoint's `path.close()` then returns `ClosePathError::ClosedPath`; the path
is already closed, so this is a successful final state. The fork now treats
that one error as idempotent completion. `LastOpenPath` and
`MultipathNotNegotiated` still fail the commit and close the connection.
The repeated real APDF trials and the observed race are recorded in
`docs/notes/2026-09-29-nat3-apdf-validation.md`. No wire format changes.

## MIOPUNCH-57 ordered QAD mapping delta

`Endpoint::observe_udp_mapping_sequence` accepts four locally configured
QAD destinations and contacts them one at a time through the existing noq
endpoint and its QUIC UDP socket. Each result records the resolved UDP
destination, reflected address, and local observation time; network changes
invalidate all copies. The existing `UdpObservation` remains the aggregate
candidate API. The ordered result lets the caller distinguish stable and
signed fixed-step mappings without treating the aggregate
`mapping_varies_by_dest` bit as a predictor. The method does not classify
filter behavior, select a punch strategy, or add a data socket. Its real
APDM+APDF validation is in
`docs/notes/2026-09-29-nat4-regular-validation.md`.

## MIOPUNCH-61 physical UDP traversal bindings

The existing Endpoint can create up to 256 additional OS-selected IPv4
listeners per authenticated traversal round. `socket/transports/ip/pool.rs`
owns these descriptors with a bounded 1024-socket Endpoint pool, monotonic
`LocalSocketId`, per-round registration and per-original-Connection ownership.
Individual bind failures are counted without replacement attempts; interrupted
setup retains its actual partial report. Dropping the round closes losers;
an uncommitted winner closes on cancellation, while a committed winner follows
its original connection and final Noq path disposal.

Receive metadata records the actual physical descriptor and destination IP/
port for each datagram. A winner pins that real interface IP. A private local
token encodes its socket ID inside the existing mapped four-tuple mechanism;
the transport resolves the token to that descriptor and real source IP before
sending. The token never becomes a wire source address. Explicit data-path
opening, path watchers, per-connection permits, path removal and network-change
recovery preserve this identity. Foreign connections cannot borrow pool
bindings, and private pool destinations are excluded from generic bootstrap
route caching. Shared primary IPv4 and IPv6 sockets remain Endpoint-owned.

Raw registration remains tied to the original admitted peer, connection and
round. Raw packet framing and QUIC wire encoding are unchanged from the prior
fork. The application still authenticates receipts with its original TLS
exporter and requires the original QUIC path and business gate before learning.
The native module is omitted in browser builds. Linux physical-source, TTL,
descriptor and controlled NAT proof is recorded by MIOPUNCH-61; browser
compilation verifies the native seam stays excluded, without claiming browser
UDP traversal or additional platform packet evidence.

The physical network epoch is separate from the aggregate observation report
generation. Periodic QAD reports supersede an aggregate candidate but do not
cancel an already admitted legacy phase. `UdpMappingSequence` and the legacy
fallback snapshot retain the physical epoch; actual network-change handling
increments that epoch before invalidating pending traversal bindings. The
existing short freshness test remains required when a prediction is acquired.

## MIOPUNCH-60 local assisted candidates

`Endpoint::udp_local_ipv4_candidates` pairs actual primary IPv4 socket ports
with their bound local addresses or the current interface addresses for a
wildcard bind. The bounded list excludes reflected, configured and portmapped
addresses and remains available without QAD observations. The formal connector
exchanges it as local assisted candidates, preserving sender assisted-before-
mapped order and the old basic fallback when classification is unavailable.
No additional socket, endpoint or wire-format change is introduced.

## MIOPUNCH-39: explicit UDP gateway mapping

`PortmapperConfig::NatPmp` selects only UDP NAT-PMP on the actual QUIC socket.
The root enables the existing portmapper feature; native saved resource
configuration owns local gateway consent independently of relay. The fork's
default is Disabled, preserving the previously feature-disabled/non-probing
runtime unless explicitly enabled. Upstream Enabled remains available but
PCP/UPnP are not claimed as native product capabilities by this ticket.

Mapping watcher changes update `Endpoint::addr` immediately, without waiting
for QAD. Graceful endpoint close awaits the mapping service's own release
attempt with a one-second bound; abort queues best-effort deactivation.
See `vendor/portmapper/UPSTREAM.md` for the acknowledgment and pending
NAT-PMP allocation cleanup adaptation.

## MIOPUNCH-40: native direct TCP qualification

`RelayResourcePolicy::with_custom_addresses` independently qualifies actual
custom peer addresses. Every custom endpoint now receives the existing policy
wrapper, including direct carriers with no auxiliary resource descriptor.
The wrapper checks both sender eligibility and the final poll_send boundary;
the optional auxiliary resource/use check is retained. This allows the native
saved TCP disable policy to cover explicit, cached and lookup-supplied routes
without treating a direct peer as an auxiliary relay facility.

`Endpoint::custom_addrs` explicitly exposes custom candidates without adding
them to automatic address-lookup publication. The native resource endpoint
adds only recognized TCP data candidates to its own public contact address.
Signaling-only custom addresses do not thereby acquire business disclosure or
data qualification. Inner QUIC/TLS, the shared admission gate and path selection
remain unchanged; IPv4/IPv6 framing belongs to the root TCP adapter.

MIOPUNCH-41 adds defaulted custom-transport binding and shutdown hooks.
Binding receives endpoint gateway consent and the existing resource policy;
the native TCP carrier checks them before any optional NAT-PMP request, including
when an advanced builder installs that carrier. `RelayResourcePolicy::allows_custom`
is public for that binding check. Each retained custom shutdown handler gets a
one-second cleanup attempt after QUIC draining and before actor cancellation.
Other custom carriers retain their default bind behavior and have no handler.
