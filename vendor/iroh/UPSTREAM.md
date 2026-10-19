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

This ticket originally added `control_only_alpns` and explicit `open_data_path`,
`open_control_path`, and `commit_data_path` calls. The per-peer actor suppresses
automatic candidate disclosure for restricted connections, keeps custom paths
off committed data connections, and closes their original custom path after an
established UDP path exists. Restricted dials with one explicit custom route
send their Initial to that route, even when another connection selected IP;
explicit path-open failures do not enter the peer-wide retry queue. A small `noq` packet gate supplies the required
send-before-`Established` and receive-before-QUIC admission boundary; merely
changing iroh's selected path cannot provide that boundary. An endpoint with
control-only ALPNs also disabled TLS early data and refused 0-RTT/0.5-RTT
handles. Its changed incoming return type and the ALPN-based configuration
were superseded by MIOPUNCH-95's independent restricted endpoint interface.

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
port for each datagram. A winner pins that real interface IP. MIOPUNCH-90
replaces the earlier private IPv6 binding token with Noq's native local socket
metadata. Real source IPs remain addresses; the selected socket identifies its
existing descriptor independently. Explicit data-path
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
configuration owns local gateway consent independently of relay. MIOPUNCH-98
restores the ordinary upstream Enabled default; product/probe/fixture callers
choose Disabled or saved NatPmp explicitly, preserving their non-probing
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

## MIOPUNCH-42: formal MQTT member bootstrap

`EndpointHooks::before_data_release` was a defaulted local API stage awaited by
both connect and accept after member admission, while the original connection
still has its restricted packet gate and before an application handle returns.
The native coordinator can validate and commit a permitted UDP path on that
same authenticated connection. Hook errors reject it; its cancellation guard
closes the original attempt. Hooks must not retain the supplied Endpoint.

The actor's ordinary candidate authorization accepts either its prior Control
stage or the Data stage already committed by this coordinator, and handles an
already closed signaling path idempotently. The final Ordinary stage retains
the existing data packet gate; MQTT cannot become a business fallback.
Default hooks preserve existing direct/TCP callers. This adds no QUIC wire
change, second identity, cryptographic handshake, or new congestion controller.
The configured MQTT origin tag and adapter lifecycle belong to the root crate.
See `docs/notes/2026-10-02-mqtt-bootstrap-validation.md` for the controlled
dual-NAT evidence and declared compatibility boundaries.
MIOPUNCH-93/94 migrated these consumers to independent admission; MIOPUNCH-95
removes the unused hook stage while retaining the same protected path work.

## MIOPUNCH-43 TCP candidate retirement delta

`Endpoint::retire_control_path` retires one failed custom route on its exact
restricted original connection. The per-peer actor records the route, filters
queued/draining packets, closes pending as well as established noq paths and
retains the exclusion when normal candidate admission installs its data policy.
Other connections and candidates remain independent. This lets the native TCP
coordinator release losing physical circuits without QUIC retries recreating
them during a permitted UDP fallback. No new QUIC wire format or congestion
implementation is added. Native protected TCP coordination and the actual
Linux/NAT boundaries are recorded in
`docs/notes/2026-10-02-tcp-punch-validation.md`.

## MIOPUNCH-47: connection-qualified quality maintenance

The defaulted `PathSelector::reevaluation_interval` optionally requests bounded
live-path reevaluation in the existing remote actor (250ms–30s, no catch-up
bursts). Native product policy installs a quality-first selector without
changing the upstream historical default for other consumers. The builder's
`default_path_selector` preserves an explicitly supplied advanced selector.
Selection contexts now qualify each Connection's own current paths and physical
UDP ownership; restricted control paths do not become peer data preferences or
ordinary business paths. Per-connection packet gates remain decisive.

An admission-gated sole custom contact remains the route for that attempt's
Initial, so another live control session cannot substitute its cached carrier.
Ungated raw consumers retain the historical mapped bootstrap and graceful-drain
behavior. Restricted endpoints keep their exact custom contact route through
their independent admission configuration. The UDP
path-opening API also works on a living admitted connection; the new
`open_data_transport_path` opens an explicitly qualified UDP/relay/custom data
candidate on an ordinary admitted connection. Closed connections return failure
and are never redialed. Existing custom-path test hooks use that same production
boundary. These are local API/actor changes, not new QUIC wire encoding or
congestion implementations. The root's separate protected same-session repair
coordinator and qualified matrix are recorded in
`docs/notes/2026-10-02-connection-path-maintenance.md`.

MIOPUNCH-47 review correction adds `Path::start_probe`, delegating to the
original noq Path's fresh authenticated data-packet acknowledgment probe.
Native recovery cannot report success from a stale address snapshot alone.

## MIOPUNCH-89: active CID pool configuration

The QUIC transport builder exposes the locked Noq active connection-ID limit.
`Builder::active_connection_id_limit` changes that one field through a cloned
transport configuration, preserving other caller settings and path limits.
The default remains five IDs per path; the SDK may request two for its narrow
IRC-only control bootstrap configuration. No new QUIC frame, path scheduler,
congestion controller or identity/admission rule is added here.
Automatic SDK tuning excludes installed MQTT, RTC, and data-relay/member-data
options. Native raw endpoint builders keep their caller settings; callers can
explicitly select two IDs with the generic builder API for a measured carrier.

## MIOPUNCH-52: Windows Rust-library packaging

The normalized manifest now builds only `lib`, omitting the unused `cdylib`
artifact. The debug x86_64-pc-windows-gnu build exceeded the PE export ordinal
limit (108336 exports) while producing that DLL. miopunch embeds the Rust
library, not a C ABI. The same packaging-only change is recorded for the
vendored locked `iroh-relay` 1.2.0 dependency. No Rust API or wire change.

## MIOPUNCH-92: typed restricted endpoint and independent admission

`Builder::restricted` explicitly selects `RestrictedBuilder` / `RestrictedEndpoint`.
These types share the original endpoint, TLS, QUIC, socket, registration and
router implementations; they expose no ordinary endpoint conversion or borrow.
Their Incoming, Accepting and Connecting states have no early-data entry point.
Ordinary endpoint early-data behavior remains available. M92 left the original
incoming return signature and old hook enum for MIOPUNCH-95, following the
separate Native / Browser migrations; this additive slice did not restore them.

`EndpointAdmission` receives a dedicated `ControlConnection`. It can exchange
bounded framed messages and qualify a path on the original connection without
obtaining raw streams, datagrams or business handles. The application supplies
its own policy; the library contains no miopunch membership state machine.
The control wire uses the first client bidirectional stream, a versioned
`iroh/admission/1\0` preface, length-limited control messages and a separate
bilateral delivery-ready frame. This is an explicit new application protocol,
not a QUIC/TLS wire or congestion change, nor classification of arbitrary payloads.

`DataPathPermit` captures the original connection, path, address and fresh Noq
ACK probe. Live path/resource qualification and current policy are checked
before and after legacy hooks, before readiness, and at final release. Only
then does the caller receive the original `Connection`. Control-only decisions
have separate connect/accept/router delivery and never pass a full connection
to a legacy business hook. `WeakControlConnection` provides liveness and close
authority without a public upgrade, including for later application revocation.

Typed attempts share one absolute budget through lookup, TLS, control, routing
and handoff. Cancellation, peer closure and dropping pending registration close
the original attempt. The setup budget ends at successful delivery. Endpoint
close/drop ownership remains the same as the ordinary endpoint: custom-carrier
drivers are released after their endpoint owners are dropped.

M92 retained the old hook-based path for its existing consumers; M95 removes
that path after their Native / Browser migrations.
`RestrictedRouter` reuses its accept loop and shutdown, delivering original
business handlers only after typed admission; it does not expose the ordinary
handler's `on_accepting` state. See
`docs/notes/2026-10-18-restricted-endpoint-api.md` for exact transition consumers,
public-interface tests and the bounded MQTT-control-to-UDP demonstration.

## MIOPUNCH-93 application control and Native migration

`Builder::restricted_application` explicitly selects existing application
control framing instead of the restricted mode's library preface and Ready
frames. Bounded complete-message exchanges support fixed, 16/32-bit length
prefixed and FIN-terminated messages without exposing raw streams. An owned
`ControlConfirmation` supplies the application's existing delivery agreement;
missing confirmation rejects, and cancellation drops its prepared resources.
The default `restricted` mode keeps its original framing. This preserves the
Native SDK's existing SCRAM, MUC1, admission-material and member protocols.

Cloneable `ControlHandle` provides confined metadata, path operations and
control exchanges. Its non-cloneable `ControlConnection` owner still closes
on drop; retained helpers cannot outlive that authority. Successful business
handoff retires all control exchanges before returning the original connection.
The defaulted `business_released` observer publishes application maintenance
state after final qualification and retirement. It does not change ordinary
endpoint hooks or ordinary early-data behavior.

Custom path qualification accepts a currently retained exact connection/address
seal as well as the explicitly allowed carrier types. The actor applies the
same existing qualification, resource policy and packet permit checks; this
does not authorize every address of that custom carrier. Native physical UDP
bindings remain owned by the original socket pool and connection.

See `docs/notes/2026-10-19-native-restricted-admission.md` and
`docs/evidence/native-restricted-admission-2026-10-19/README.md` for the Native
migration, fixed-counterpart interoperability and regression evidence.

## MIOPUNCH-95 ordinary early-data and hook compatibility

Ordinary `Accepting::into_0rtt` directly returns `IncomingZeroRttConnection`
again; outgoing `Connecting::into_0rtt` retains its original `Result` contract.
The ordinary `EndpointHooks` interface and exhaustive `AfterHandshakeOutcome`
are restored to the fixed official source, with only `Accept` and `Reject`.
The unused `admission_gate`, `control_only_alpns`, `before_data_release`,
expanded outcomes and their internal bridge configuration are removed.

The existing optional `EndpointAdmission` policy now identifies restricted mode
internally. It still disables TLS early data, restricts connection registration
and pins an explicit sole custom contact route. Restricted states expose no
ordinary early-data or object-conversion entry points. Their independent
`AdmissionOutcome`, control exchanges, data-path permits, legacy-hook veto,
bilateral confirmation, revocation and bounded cleanup remain in the shared
implementation. Ordinary endpoints keep their original early-data and hook
timing. No QUIC/TLS algorithm or application wire protocol changes.

## MIOPUNCH-98: ordinary portmapper defaults and explicit caller intent

The ordinary `PortmapperConfig::default()` again selects Enabled. This does not
activate the Cargo feature for callers that disabled it, or install a native
service in Browser builds. The saved product resource assembly still selects
Disabled/NatPmp before binding, including advanced builders and TCP ownership.
Direct probes and ordinary local test fixtures declare Disabled explicitly;
fixed original consumers retain their default and run in an isolated gateway lab.

A default-feature build also exposed a missing cfg on the restricted endpoint's
`custom_addrs` delegate. It now uses the same unstable-custom-transports condition
as the ordinary method, so callers do not need that new feature to compile.
The portmapper crate's protocol, allocation ownership, pending cancellation and
bounded acknowledged release fixes are unchanged. Close returning is distinct
from a gateway-confirmed deletion; the owned wire receipts record both.

## MIOPUNCH-99 pkarr redirect compatibility

Ordinary native pkarr clients retain reqwest's original redirect policy. The
existing client builder now has a public constructor; new and build share their
implementation. Client, publisher and resolver builders accept an explicit
resource policy, disable native redirects and check lookup permission before
requests and publication retries. Browser constrained requests stop before Fetch.
Standard lookup builders inherit explicit endpoint policies without discarding
additional caller restrictions. Explicit-policy registries reject external
services without a constrained-access declaration, including prebuilt ordinary
pkarr services; Arc/filtered wrappers preserve it. Ordinary registry behavior and
old AddressLookup implementations remain available. Third-party declarations
are contracts, not a sandbox around independently started tasks.

See `docs/notes/2026-10-19-pkarr-redirect-compatibility.md` for the unchanged
original consumer, private-builder test adapter, real redirects, product permission
negatives and Chromium boundary evidence. No dependency or wire format changes.


## MIOPUNCH-101: preserve peer selectors; opt into connection selection

`Builder::path_selector` retains one peer-aggregate callback per selection
cycle. Applying that decision never calls the old selector again per connection.
`connection_path_selector` explicitly registers `ConnectionPathSelector`, using
that original connection's live, qualified paths. The callback's context exposes
its live `connection_id`, preserves address-only `current()`, and provides
socket-aware `is_current` and candidate `local_socket_id` on native targets.
Retained results are revalidated against the original connection's current
qualified paths before application; a peer hint cannot transfer another
connection's custom association or physical socket permission.

The last explicit selector registration wins. `default_connection_path_selector`
preserves either explicit registration, so saved product defaults do not override
an advanced caller. Native and browser product policies use the new entry and
share the existing quality scorer. Ordinary peer defaults keep the historical
biased-RTT scorer; restricted connections without an explicit scoped selector
use that same scorer on their qualified paths. Reevaluation stays opt-in and
bounded. No socket ownership, generation, packet gate or QUIC wire contract is
changed. See `docs/notes/2026-10-19-selector-compatibility.md` for evidence.

The ordinary path-guard audit (MIOPUNCH-103) retains the existing final packet,
application and native socket guards. Known ordinary IP/relay paths are
permissive; unbound legacy custom senders do not acquire connection-owned data
qualification. The native socket check is retained on every connection to reject
wrong-owner arrivals on pool sockets introduced later. Ordinary paths with no
local socket ID do not perform a pool lookup. Measured callback/method entries,
actual streams, buffered-send rejection and scope limits are recorded in
`docs/notes/2026-10-19-path-guard-compatibility.md`; zero custom-policy entries are
not a claim of zero internal predicate cost.
