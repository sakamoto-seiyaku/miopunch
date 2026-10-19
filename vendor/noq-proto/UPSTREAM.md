# noq-proto packet-source accessor

Based on the published crates.io `noq-proto` 1.3.0 package, registry SHA-256
`7c1e5b6fe668491eca022f745a0a9402585626c73a7b839b3424ace15d6a9c8f`.
The package's `.cargo_vcs_info.json` names upstream commit
`c1f411562e6078852749b8bcf1190523096a107f`, path `noq-proto`.

MIOPUNCH-21 changes `src/shared.rs` and removes one trailing space in
`src/config/transport.rs`. `ConnectionEvent::datagram_remote_addr`
exposes the transport address already held by an incoming event, allowing
`noq` to enforce a connection's path policy before QUIC processes that event.
It does not change packet encoding or protocol state. The dependency is
selected by the root `[patch.crates-io]` entry.

MIOPUNCH-61 adds `ConnectionEvent::datagram_network_path` for the complete
source and local tuple, and a read-only iterator over all protocol-retained
network paths. The network-change hint can retain the real IP of a live binding
after the caller validates its interface. MIOPUNCH-90 carries socket identity
separately and removes the application binding token. These
accessors support the physical-socket ownership gate and final descriptor
release; they do not change QUIC packet encoding.

MIOPUNCH-47 adds fresh data-path acknowledgment probes. The protocol helper
atomically captures the next data-space packet number and queues an ordinary
PING, then reads the existing largest authenticated per-path ACK watermark.
Closed connections and abandoned paths are rejected. The async wrapper owns
the original Path and opaque watermark, preventing callers from accidentally
checking another connection or path. No QUIC frame encoding, ACK processing,
congestion control or lifetime algorithm changes.

## MIOPUNCH-89: configurable active CID pool

`TransportConfig::active_connection_id_limit` advertises two through five
active peer IDs per path, retaining the upstream default of five. The existing
CID receive window uses the advertised limit when accepting new IDs and
bounding retirement ranges; excess IDs still trigger the original connection
ID limit error. Queue capacity, path limits, CID replenishment, frame encoding,
loss detection and congestion control remain upstream behavior. Tests cover a
two-ID window, reserved/retired IDs, repeated rotation, and eight concurrent
paths. The IRC SDK tuning and measured qualification are recorded separately.

## MIOPUNCH-90: local socket identity on protocol paths

An opaque endpoint-local `LocalSocketId` augments private `FourTuple` state;
existing constructors leave it unspecified. Equality, path reuse and validation
distinguish sockets even when real addresses match. New `SocketTransmit` and
`poll_transmit_with_socket` preserve the selected socket for ordinary,
previous-path and off-path packets while the original `Transmit` fields remain
unchanged. Explicit-bound drivers must use the new poll rather than discard
metadata. A retained path keeps its local socket; receiving on a different socket cannot
move it or inherit its proof. Another socket requires an explicit new path on
the same connection. Peer-address migration retains its existing rules, and
bound probes reject previous generations. None of this metadata becomes a QUIC wire field.

## MIOPUNCH-102: explicit Backup ACK routing

Ordinary connections and `ping`/`ping_path` retain upstream cross-path ACK
routing. Calling the fork's `start_path_probe` explicitly prefers that local
binding for its own Backup ACKs until replacement or close; other paths are
unchanged. Both peers must opt in locally for bidirectional fresh confirmation
when an Available carrier is blackholed. New and migrated bindings start with
the ordinary policy. Clearing or changing the local IP during in-place recovery
also resets the preference; a notification that preserves the tuple does not.
No new transport config or wire negotiation is needed.

This is a selected probe behavior, not a universal defect fix: fixed real Noq
consumers show that always routing Backup ACKs locally can delay ordinary stream
completion and actual loss recovery when the backup return path is slow. Packet
and ordinary-frame permissions, authenticated ACK processing, connection close,
loss detection, and congestion control are unchanged.

## MIOPUNCH-103: recheck buffered application transmits

Each actual encoded Data-space frame records whether it requires application
permission, preserving the existing receive-side validation/control exceptions.
The private bit is combined across coalesced packets and GSO datagrams, then
retained on SocketTransmit. Connection::application_data_transmit_permitted lets
the async driver recheck the current predicate immediately before each sender
attempt, including a retry after Pending. The bit resets for each new transmit;
PING, ACK, path validation, observation and close packets remain eligible while
ordinary data is forbidden. A mixed batch requires permission for the whole
batch. The original Transmit fields and poll_transmit return contract, socket
metadata, wire encoding, loss recovery and congestion algorithms are unchanged.
Unconfigured predicates remain permissive. See the M103 compatibility note for
the pre-fix real plaintext leak, fixed original consumers and regression scope.

## MIOPUNCH-104: explicit diagnostic runtime contract

The existing doc-hidden packet_witness module is compiled in ordinary Native and
WASM graphs; doc-hidden is not test-only or feature isolation. Installation is
explicit, process-wide and once-only. finish disables observation after the
caller has quiesced producers; it is not a join barrier and does not permit
reinstallation. Synchronous callbacks cannot wait or re-enter a connection lock.
Recursive emit, observer unwind panic and bounded field/record/byte failures
mark loss and disable capture, without granting packet or application permission.

Default paths retain atomic enabled checks and some ring STREAM scalar snapshots;
enabled paths add hashing, copies, allocation and callback work. Some encoder
hash observations retain their ring cfg. Budgets count fields, not total heap or
physical sends. No new diagnostic feature or production logic change is made by
this audit. See docs/notes/2026-10-19-packet-witness-opt-in.md for actual off/on,
driver callback failure, bounds, Native/WASM provenance and proof limits.
