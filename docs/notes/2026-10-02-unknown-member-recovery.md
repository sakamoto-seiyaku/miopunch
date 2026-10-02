# Native startup recovery through an unknown member

[MIOPUNCH-46](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/0148dccb-a7a8-4e83-9235-75e66348b0ce)
connects the previously validated MQTT candidate profile to durable native nodes,
the configured endpoint, its address book and the existing control-history
LogSync consumer. The parent specification's node, Endpoint, Connection and
stream seams remain the acceptance boundaries; no product UI is introduced.

## Recorded integration choices

`NativeNetwork::resume_network(bind_ip)` is the startup operation. It binds the
saved identity and allowed resources, starts online candidate answering, discovers
candidate identities and routes without requiring business traffic, and attempts
source-scoped recovery. `recover_network(&mut ResourceEndpoint)` retries on the
same endpoint after waiting, a new clue or a resource/network change. An online
node's router dispatches the new restricted recovery ALPN to
`serve_network_recovery`; callers do not manually construct query topics, proofs
or a minimal membership proof package.

Candidate discovery uses MQTT origins explicitly allowed for **both Lookup and
Signaling** in saved configuration. No new public default is enabled. At most
four qualified origins are queried concurrently; one failing origin does not
block a live one. Each query retains the validated random nonce, non-retained
QoS 0 response rules, two-second window and sixteen-candidate bound. The global
attempt set is also capped at sixteen and removes exact duplicate hints. These
are the bounded implementation profile, not approved global latency targets.

The existing `MDQ1 || nonce` query remains twenty bytes. Configured recovery uses
`MDR2 || nonce || node_id || origin_tag/delivery_token || IPv4 || UDP_port`, ninety
bytes in total. The route keeps the complete thirty-two-byte configured CustomAddr;
its public sixteen-byte origin tag is checked against the actual qualified MQTT
origin before use. Legacy `MDR1` and sixteen-byte probe routes remain compatible
with the earlier validation entry. The explicit Legacy/OriginQualified profile
rejects unsupported route widths before any broker access. This is a versioned extension of that profile,
not automatic unknown discovery through IRC or arbitrary carriers.

Each online responder checks its current admission generation, local membership
and live subscribed route for every query. Withdrawal, shutdown, local revocation
or credential changes stop old-scope answers. ResourceEndpoint owns responder
lifetime and aborts them when dropped or closed. Credential changes require a
fresh configured binding to answer in the replacement namespace. Disabled lookup
never gets reenabled by an otherwise live signaling resource. The shared endpoint
installs its temporary MemoryLookup only when qualified discovery resources exist;
LAN-only/default startup behavior is preserved.

A candidate enters this endpoint's temporary address book as an untrusted route
hint. Contact uses the existing ResourceEndpoint timeout and per-identity backoff;
an actually changed route can trigger a fresh attempt. The same hint does not
reset an existing failed-contact backoff. The TLS target is the candidate's exact
EndpointId; route/topic metadata never supplies membership authority.

The recovery ALPN is Bootstrap scope. The existing AdmissionGate completes
session-bound SCRAM-SHA-256-PLUS and rejects wrong credentials or known revoked
identities. The source requires both identities to be current members in its own
applied view before releasing any history. A returning node may not yet know the
source's membership; after proving the admission secret it validates the hinted
UDP path on this original authenticated Connection and permanently retires its
signaling paths. This retains the earlier unknown-bootstrap data-path profile.
History access additionally requires the restricted connection to have only IP
paths. A failed or unavailable path produces Waiting, not a data or business handle.

The normal offline-replica Manifest, Snapshot, real p2panda LogSync protocol and
durable management consumer are reused. The consumer verifies original signed
records, pinned network/Create, bodies and causal dependencies, updates the local
view and reports saved versus applied IDs. Neither the full-history CBOR exchange
from the old experiment nor an independent synchronization system is copied.
`Ready(ReplicaOutcome)` requires complete saved/applied source scope and both
currently authorized endpoint identities. `WaitingForDependencies(ReplicaOutcome)`
retains actual source progress for later repair. Transport Done alone is insufficient;
`Waiting` covers no qualified candidate/path or rejected authentication/authorization.
These results do not claim globally latest state or require the original author,
old approver or every other member to come online.

Only the new recovery ALPN adds a final acknowledgement to the existing replica
receipt. The provider retains its original handle until the receiver closes after
observing that acknowledgement, within a twelve-second close bound. This fixes the
observed race where dropping the provider or immediately closing the receiver
lost the last frame. The existing member replica wire protocol is unchanged.
Recovery deliberately closes this restricted exchange when finished; it does not
resurrect an old application session. Verified IP contact information then remains
in the same endpoint's address book for ordinary member-protocol dialing.

## Qualification and privacy limits

The actual supported evidence here is native Linux with owned Mosquitto brokers,
independent node identities and SQLite stores, real iroh QUIC/TLS, real SCRAM,
loopback IPv4 UDP qualification, LogSync, persisted authorization and controlled
member business streams. The original administrator is shut down throughout the
recovery check. The retained physical connection performs the history exchange;
normal member traffic is a separately authorized later Connection.

The independent MQTT observer verifies the exact twenty/ninety-byte discovery
fields, full configured route and expected IP/port, and sees only allowed public
query/reply fields or QUIC packet envelopes. Secret bytes are absent. This is a
bounded observation, not a cryptographic audit or anonymity claim. Facilities
still observe namespace, nonce, node key, route, IP/port, sizes and timing.

No unknown-member NAT success rate, relay-only unknown upgrade, IPv6 discovery,
public broker availability, automatic IRC discovery, arbitrary platform behavior,
transparent old-session recovery, service ACL, KCP, TUN or client UI is claimed.
The earlier qualified MQTT/member/TCP/IRC carriers remain their own capabilities;
this ticket does not infer their unknown-discovery combinations. Unchanged slow
NAT matrices were skipped under the user's explicit test policy.

Ordered acceptance, retained RED/diagnosis results and affected short regression
receipts are indexed in [the evidence README](../evidence/mio46-recovery/README.md).
