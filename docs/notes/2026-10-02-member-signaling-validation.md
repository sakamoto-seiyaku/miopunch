# Existing members as a signaling carrier

MIOPUNCH-44 adds one intermediate member to the formal native connection entry.
It uses the public seams approved in MIOPUNCH-14: persistent nodes, real member
admission, Endpoint, Connection and streams. Acceptance runs on Linux loopback
with three independently identified members and real TLS/QUIC. This ticket does
not claim a new NAT topology, browser runtime, member data relay or multi-hop
routing capability. The existing path coordinator's NAT support remains bounded
by the [MQTT](2026-10-02-mqtt-bootstrap-validation.md) and
[TCP punching](2026-10-02-tcp-punch-validation.md) evidence.

## Formal assembly and shared connection behavior

Configure and save resource permissions on `NativeNetwork` as usual. Construct
`MemberSignalingTransport::new(B_id, A.admission_gate())`, add it to the advanced
builder, and bind using `bind_configured_endpoint_with`. After the existing A-B
member connection is available, attach its reserved signaling stream through
`transport.attach`. B dispatches that stream to
`MemberForwarder::accept_link`, using its own node's admission gate. Repeat for
C-B. This is SDK assembly; a protocol's stream dispatcher must reserve the
initial bi-stream for signaling before competing application stream consumers
start. The adapter subsequently owns only that stream. It never polls for
additional application streams on the A-B connection.

Obtain the adapter's public delivery address with `transport.address()`. An
`EndpointAddr` pins the actual A-C peer identity independently of that address.
Call the ordinary `Endpoint::connect` (or `ResourceEndpoint::connect`) and
`Endpoint::accept`; those are also the MQTT connection interfaces. No business
handler opens its own candidate negotiation or creates a second A-C connection.

The former `mqtt_bootstrap` module is now `message_bootstrap`. Formal startup
installs the existing coordinator even when no MQTT service is configured. It
recognizes precisely the MQTT and member signaling transport IDs, verifies
membership with the existing gate, exchanges protected capabilities, and runs
the existing TCP/UDP path coordinator. The original connection's data-release
gate withholds ordinary protocol delivery until a permitted data path exists.
The new transport ID is never added to the member data-carrier allowlist.
Explicit older `bind_endpoint[_with]` assembly retains its existing behavior.

B is a signaling helper by default when this service is installed. That requires
live Member scope on both outer links; Bootstrap scope alone is insufficient.
The service supplies no business relay capability and does not enable one.
With A-C new-path permission withdrawn and no TCP/relay configured, the real
protected control exchange fails clearly rather than returning a business
connection. A-B and B-C remain usable. No changes were made to authorization
records, membership proof, resource consent, congestion controllers or NAT
algorithms.

## Recorded implementation decisions

The existing mature endpoint security and attempt engine protect the inner
A-C exchange. B forwards opaque QUIC datagrams through an already admitted
member connection, rather than implementing another handshake or attempt state
machine. Candidate addresses, admission proof and business streams remain
inside the inner endpoint session. B can see outer member identities, delivery
tokens, packet sizes and timing. Public QUIC handshake metadata is not described
as encrypted, and packet-marker checks are not a cryptographic audit.

Version 1 uses a 48-byte `CustomAddr`: the expected forwarding member's 32-byte
EndpointId followed by a random 16-byte delivery token. The stream's initial
registration is `MIM1` plus that token; B replies `MIM1`. Each subsequent frame
contains a 16-byte target token and a big-endian u16 length followed by one
opaque datagram. On delivery B replaces the token with the authenticated
sender's registered token. A sender cannot choose a different source label.
Neither invitations nor admission secrets enter this envelope. An address from
a different forwarding member fails the adapter's sender validation.

Each datagram is limited to 2048 bytes before allocation. Per-link incoming and
outgoing queues hold at most 64 packets. B registers at most 16 links and rejects
a repeated token or second link for the same authenticated member. These are
implementation safety bounds, not product capacity or performance promises.
Full queues drop datagrams; the existing QUIC engine handles retransmission.
Reading and writing have independent loops, so a pending partial frame cannot
be discarded by selecting an outbound queue event.

Callers retain the two stream `SignalingLease` handles. Each lifetime is greater
than zero and at most 300 seconds, and the establishment reads/writes have
bounded waits. Expiry, stream interruption, endpoint shutdown or lease drop
removes the corresponding delivery registration and advertisement. These
operations end the reserved stream, not the whole outer member connection.
An attached adapter is one-shot; a new attachment after expiry uses a new
adapter/delivery token. The iroh engine owns each inner attempt and rejects
duplicate QUIC packets without creating duplicate delivered connections.

## Validation and limits

The reproducible entry is `scripts/check_member_forward.py --output <new-dir>`.
It runs the first real check before the priority failure check, then isolates
the added expiry, interruption, duplicate, identity and shutdown conditions.
`--short` additionally runs the affected existing native/MQTT checks using
owned brokers, compile checks and formatting. See the
[evidence index](../evidence/mio44-member-forward/README.md) for actual receipts,
initial failures and authoritative final results.

The positive test offers only C's member delivery route to A, then observes the
same authenticated connection's protected bootstrap and validated UDP path,
and exchanges actual A-C business streams. The failure test independently
withdraws A/C's runtime permission for new UDP paths while keeping the existing
outer links alive. No other inner data mechanism is configured. This verifies
the permitted-path boundary, not a new physical NAT failure result.

The existing `member-signaling-test-hooks` feature deliberately duplicates real
opaque forwarded datagrams and retains a bounded packet observation for that
test. Normal builds contain neither duplication nor packet capture. Real TLS
target rejection, membership checks and QUIC handling remain active. Existing
unchanged slow NAT matrices are skipped under the user's standing test policy.
