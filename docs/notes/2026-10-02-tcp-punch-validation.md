# Coordinated TCP punching for formal native nodes

MIOPUNCH-43 integrates TCP traversal with the existing formal
`NativeNetwork::bind_configured_endpoint[_with]`, MQTT member proof and fork
`before_data_release` stage. Normal callers still use `ResourceEndpoint::connect`,
`Endpoint::accept`, `Connection` and ordinary streams. The original iroh TLS/QUIC
session becomes the data connection; the transport does not grant membership.

## Recorded configuration and support boundary

An existing saved `tcp_listen` plus an explicitly allowed Observation resource
such as `stun+tcp://100.64.27.1:3478/` enables this increment on Linux IPv4.
`stun+tcp` is a local resource-origin descriptor, not a claim to implement every
standard STUN URI form. Only literal IPv4 observer origins with a nonzero port
are selected, at most four per attempt. These observers remain independent from
HTTPS QAD and native relay resources. Bundled defaults are unchanged. Local
Observation disables, TCP disables and exact candidate disables apply; gateway
mapping remains separately disabled unless explicitly enabled by its existing
configuration. No STUN result grants reachability or authorization.

The actual supported dual-NAT fixture uses Linux kernel MASQUERADE with
port-preserving mappings, a filter that silently drops unsolicited WAN SYNs,
and fresh peer-flow state supporting TCP simultaneous open. Each listener,
observation socket and peer dial use the same real IPv4 port. Linux requires
both SO_REUSEADDR and SO_REUSEPORT for this post-listen binding sequence; the
initial SO_REUSEADDR-only attempt actually failed with AddrInUse. Wildcard and
explicit private-interface bindings both ran behind two kernel NAT gateways.
There is no static TCP DNAT, NAT-PMP lease, public peer listener, UDP data path
or native relay in the TCP-only success cells.

| Platform / mode | Actual evidence and enabled scope |
| --- | --- |
| Linux x86_64, IPv4 wildcard | Real dual-NAT TLS/member admission, replica sync and TCP business |
| Linux x86_64, IPv4 private interface | Same real dual-NAT transfer from the bound interface/port |
| Linux IPv6 | Existing direct TCP regression runs; punching is disabled by the IPv4 guard |
| Android, Windows, macOS, browser | TCP punching is disabled / unclaimed in this increment; no runtime claim |

Host kernel: Linux 6.6.114.1-microsoft-standard-WSL2, x86_64. The owned Docker
fixture uses `miopunch-nat61-lab:bookworm`, Python's independent TCP Binding
observer and real Mosquitto. This is not arbitrary-NAT, hardware-router,
cross-operator or success-rate evidence.

## Protected coordination and lifecycle

After member proof, peers exchange a three-byte protected capability frame
(version1, TCP boolean, UDP boolean). Both must permit a mechanism. This avoids
misreading a TCP candidate as UDP control when only one endpoint enables TCP.
A TCP-capable pair observes from its actual listener port, retains the observer
socket through validation and exchanges an IPv4 candidate on an encrypted QUIC
stream (`MIOTCP01`, one availability byte, four address and two port bytes).
Both peers exchange observation availability, candidate permission and final
validation status before leaving that phase. One-sided observer failure or an
exact local candidate disable therefore selects permitted UDP at both ends,
rather than mixing TCP and UDP control frames or releasing a unilateral winner.
STUN uses one bounded,
unauthenticated RFC8489 Binding transaction, with a random transaction ID,
response/header/attribute validation and a 1024-byte response ceiling. It never
sends node credentials, admission secrets or member/business records.

The fork opens the custom path on the same restricted connection. Its existing
final data allowlist closes MQTT and permits the existing TCP data carrier only
after a validated TCP path exists. A failed TCP candidate is retired on its exact
original connection, including opening paths and queued/draining packets; later
admission cannot revive it. This prevents TCP retransmits recreating a loser
circuit while an allowed UDP path takes over. No MQTT business fallback is added.

Duplicate attempts are keyed by authenticated peer and ALPN. Simultaneous
opposite directions prefer the lower node's dialing connection; same-direction
ties use the same TLS exporter token at both ends. A bounded 200ms settling
window precedes observation. A live released winner rejects another session,
including contact that starts on a cached TCP path instead of MQTT. Different
ALPNs retain independent scopes. Circuit leases retain actual sockets through
connection lifetime, cancel pending dials/streams and preserve another owner's
share. Incoming and outgoing circuits claim one bounded packet queue; the
private TCP preface remains an untrusted route hint, not an identity proof.
Endpoint close also shuts TCP before releasing only its own optional mapping.

TCP path validation is bounded to six seconds after candidate exchange;
observer operations have one-second bounds. Existing ResourceEndpoint contact
bounds/backoff remain in force. TCP is tried when both endpoints explicitly
support it; its bounded failure can continue through mutually permitted UDP on
the original connection. Existing iroh quality selection and BBRv3 are used.
No custom congestion controller, KCP, UI, ACL or general recovery system is added.
This increment validates UDP as the alternate; it does not claim a new
MQTT-first native-relay upgrade.

## Failure and retry limitation

Destination-dependent peer port rewrites fail within the bound without releasing
business; repeated contacts back off. Canceling the original attempt releases
real SYN-SENT sockets even while the endpoint remains alive. A new attempt has
a new stable connection ID and the same durable node identity.

Immediate reuse of a canceled peer tuple is not guaranteed by the declared NAT.
The diagnostic capture showed its old SYN conntrack sequence state treating a
new passive SYN-ACK as invalid and forwarding it with a private source address.
This was not an SDK socket leak or identity error. Changing only the observer
close behavior did not fix it and was reverted. The successful retry fixture
models expiry of only the owned peer conntrack flows, preserves broker/observer
flows and then establishes a fresh real TCP connection. It does not delete
foreign gateway state or claim the production client can expire NAT entries.
The prior bounded failure and capture remain in the evidence; original closed
connections are never revived.

## Reproduction and verification

The public seams are those already approved in the parent specification:
formal nodes, real admission/stores, Endpoint/Connection/streams, socket access,
identity, cancellation and controlled network faults. Run one cell in a new
output directory:

```sh
rtk proxy python3 -B scripts/check_tcp_punch.py --case positive --output NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_punch.py --case simultaneous_cancel --output NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_punch_short.py --suite --output NEW_DIRECTORY
```

The [evidence index](../evidence/mio43-tcp-punch/README.md) distinguishes genuine
behavioral RED/GREENs, fixture/compiler errors and unsupported gateway state.
Final affected checks omit already verified unchanged slow NAT matrices under
the user's standing test policy. Raw stores and keys stay in owned temporary
container directories; cleanup removes only those processes and namespaces.

Protocol references: [RFC8489 STUN](https://datatracker.ietf.org/doc/html/rfc8489)
for Binding/TCP framing and XOR-MAPPED-ADDRESS, and
[RFC6544 section5.2 / appendixB](https://www.rfc-editor.org/rfc/rfc6544)
for the simultaneous-open/socket-reuse context. No full ICE conformance or
new cryptographic handshake is claimed.
