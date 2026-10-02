# Explicit ordinary-member end-to-end data relay

MIOPUNCH-48 implements its [current ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/337935b7-cee1-49e9-8685-0f76ef9c61b4)
and [parent MIOPUNCH-14](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
Current live ticket bodies define this scope. The previous path-maintenance
note's historical suggestion that 48 was a background scheduler does not define
this implementation. No architectural-scan suggestions became new requirements.
The approved seams remain persisted nodes, Endpoint, Connection and streams,
with real membership records, TLS/QUIC and controlled actual carriers/faults.

## SDK assembly and explicit permission

B constructs `MemberDataRelay::new(B.admission_gate())`. This service is disabled
by default. Only `set_enabled(true)` explicitly permits data forwarding. Installing
`MemberForwarder` for signaling or allowing relay consumption in resource settings
does not enable this service. B need only be an ordinary current member: no new
administrator, relay role or signed membership permission is introduced.

A and C each construct `Arc<MemberDataTransport::new(B_id, node.admission_gate())>`,
install it with `Builder::add_custom_transport`, and use
`NativeNetwork::bind_configured_endpoint_with`. After obtaining ordinary admitted
A-B and C-B member connections, the stream dispatcher reserves one bi-stream per
adapter: the client calls `transport.attach`, B calls `relay.accept_link` for that
same outer connection. Keep their returned `SignalingLease` handles alive. This
shared lease type owns a reserved packet-carrier stream, whether signaling or
explicit data; it never consumes unrelated application streams. As in 44, a
protocol dispatcher must reserve those streams before competing stream consumers.

A-C use the ordinary `Endpoint::connect`/`accept`, `ResourceEndpoint::connect`
and Connection/stream APIs. The target `EndpointAddr` pins C's independent identity
and can offer `C_transport.address()`. `ResourceEndpoint::addr` advertises this
recognized data carrier after attachment. The inner A-C iroh connection retains
its own authenticated TLS and ordinary member-admission checks. B's code only
reads/writes opaque datagrams on the outer connections and has no inner Connection,
TLS session, exporter, traffic-key or plaintext-stream handle.

Saved `ResourceConfig.member_data_relay_disabled` forbids consuming member data
routes, independently of signaling. It defaults false for old saved resource files;
this allows an explicitly supplied member data candidate, not operating a service.
Global `disabled_uses: DataRelay` also forbids consuming these routes. Filtering
applies to advertised/repair candidates and the fork's actual custom sender, so an
explicit or cached address cannot bypass a disable. Native startup registers the
separate data transport ID with the existing admission gate only when resource
consent permits it. Signal IDs never enter that data allowlist.

## Recorded route and budget choices

The existing 44 packet engine now has private signaling/data carrier modes; the
public services remain distinct. Data uses custom ID `0x6d696f646174` (miodat) and
`MID1` registration, while existing signaling keeps its miomem ID and `MIM1`.
A CustomAddr has B's 32-byte identity and a fresh 16-byte route token. A frame has
a 16-byte destination token, big-endian u16 length and one opaque QUIC datagram.
B binds each registration to its actual authenticated outer member and supplies
that registered source token when delivering; a sender cannot supply a different
source label. Unknown/self destinations drop. Duplicate tokens or a second active
registration for one member fail. Inner TLS independently rejects replacing the
intended A-C identity with B or another endpoint, even when the delivery route is
otherwise valid. Tokens and routing metadata are not identity credentials.

The existing bounds remain 2,048 bytes per datagram before allocation, 64-packet
queues per direction/link and at most 16 active member registrations. Data adds
hard per-link ingress and egress ceilings of 1 MiB opaque datagram bytes and 4,096
packets per one-second window. Each over-budget or full-queue datagram drops;
existing QUIC retransmission/congestion control handles loss. These are reversible
initial safety ceilings, not Internet throughput, precise physical-link shaping,
mobile power or capacity promises. No custom congestion controller is added.

Each route lifetime is positive and at most 300 seconds. Registration's exact
expiry also governs its forwarding task; five-second establishment waits do not
extend a registered route. Data tokens are single-use within a service instance.
The bounded 4,096-token registration history rejects retired tokens and fails
closed at exhaustion, rather than evicting tombstones and reviving stale addresses.
A caller may explicitly replace the exhausted service after retiring its owned
links; there is no automatic permission reset or advertised route resurrection.
These ephemeral circuits are not saved as durable node contacts.

A data outer link must not itself contain a member data-circuit path, including
later path changes. Live member authorization is checked at registration and
before packets are routed/written. This maintains one intermediate member,
rejects Bootstrap-only outer sessions and prevents an implicit nested circuit.
Normal direct/TCP/native-relay outer connections remain allowed. More hops and
new member permission roles are outside this ticket.

## Withdrawal and original-Connection lifetime

`set_enabled(false)` synchronizes permission change with registration, removes
active routes and wakes every old forwarding task. An immediate re-enable still
retires those old leases; new adapters/tokens are required. Dropping a lease,
expiry, reserved-stream interruption, outer connection close, endpoint shutdown
or received valid member removal ends the affected registrations/advertisements.
Stopping a reserved stream does not close its otherwise valid outer member link.
Membership removal continues to use the existing signed-state/connection gate.

The data adapter's custom sender becomes unavailable when its owned stream ends.
The existing iroh remote actor now retires exact unavailable custom addresses on
each admitted ordinary Connection before scoring them. It uses the same real
sender qualification as actual sending and preserves that Connection's data
allowlist and retired-address packet filter. Raw and restricted consumer lifecycle
is unchanged. This check belongs to existing event/interval maintenance, not a new
product recovery loop. Quality uses the existing RTT/hysteresis policy, with
member data paths recognized as relays for `PreferRelay` and as non-direct for
`PreferDirect`. Resource qualification still wins over preferences.

With an existing allowed backup, closing the retired path keeps the exact A-C
Connection and original stream. When the last physical carrier disappears, the
last path may remain in retained statistics, but is excluded from qualified
candidates and its packets cannot be sent. Actual QUIC lifetime determines clear
Connection/stream failure; no re-dial is presented as continuous recovery. A
separately live signaling service remains restricted and cannot carry queued
business or turn into a data route. Explicit new-carrier recovery remains the
existing 47 operation; no new background member scheduler is added.

## Evidence and qualified limits

The ordered rerunnable entry is `scripts/check_member_relay.py --output NEW_DIR`.
It discovers real test cases, executes the first real check then priority failure,
and requires exactly one executed test for each selector. `--case NAME` narrows
one condition; `--suite` adds affected short integration/type/WASM/format checks.
See the [evidence index](../evidence/mio48-member-relay/README.md) for actual exits
and separate review reports.

Tests use independent persisted native nodes and SQLite, real signed admission
and management records, real TLS/SCRAM/QUIC, existing loopback native TCP outer
links and an explicitly allowed actual UDP backup. The fixture demotes B to
ordinary read membership and keeps A a daily administrator for the B-removal
case. No elevation remains active during forwarding. The first test transfers
actual A-C business with UDP physically absent. The priority failure retires B's
only data carrier and observes the old session fail while outer links still work.

Security evidence combines real A-C endpoint identity checks, matching A-C TLS
exporters at A/C but different exporters on B's independent A-B/B-C sessions,
a real wrong-target TLS failure, route-denial observations, and opaque actual
packet forwarding. The exporter bytes are neither logged nor shared. Absence of
plaintext markers is supplementary, not the security argument or a cryptographic
audit. B can observe outer identities, delivery tokens, public handshake metadata,
packet sizes and timing; content protection does not imply anonymity.

Additional conditions include default disabled service, Bootstrap insufficiency,
disable/disconnect/signed B removal/stream interruption preserving both original
IDs and the same stream on UDP backup, route expiry and immediate re-enable,
unknown destinations and retired-token rejection, real rate drops/oversized
frames, consumer disable at actual send, one-hop rejection, and queued business
with a separately live signal service. Existing 44 signaling-without-data tests
remain affected regression coverage.

The initial first check found missing data APIs. An early sole-path fixture still
had automatically discovered UDP backups; it was corrected to TCP-only outer
links before making a sole-carrier failure claim. The actual backup RED then
showed `PreferRelay` retaining an unavailable member route and blocking the same
stream for five seconds. Exact sender qualification/Connection-local retirement
fixed that case, and its GREEN keeps the original IDs and stream. The last IRC
regression initially lacked its owned-server environment; the runner now supplies
real usable/rate-limited Ergo instances and resumes only the remaining checks.
The failed harness receipt is retained. No NAT,
congestion-control, admission-proof or membership-conflict algorithm was changed.
Unchanged slow NAT matrices are skipped under the user's explicit instruction.
This evidence does not qualify arbitrary NAT combinations, browser member
circuits, public origins, other platforms, multi-hop, a full service ACL, TUN,
product UI or a runtime plugin system; browser checks are compilation only.
