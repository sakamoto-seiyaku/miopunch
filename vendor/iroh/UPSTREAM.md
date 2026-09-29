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
