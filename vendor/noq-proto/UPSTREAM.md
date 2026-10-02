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
network paths. The network-change hint can retain an application binding token
for recovery after the caller validates its underlying real interface. These
accessors support the physical-socket ownership gate and final descriptor
release; they do not change QUIC packet encoding.

MIOPUNCH-47 adds fresh data-path acknowledgment probes. The protocol helper
atomically captures the next data-space packet number and queues an ordinary
PING, then reads the existing largest authenticated per-path ACK watermark.
Closed connections and abandoned paths are rejected. The async wrapper owns
the original Path and opaque watermark, preventing callers from accidentally
checking another connection or path. No QUIC frame encoding, ACK processing,
congestion control or lifetime algorithm changes.
