# noq packet-admission fork

Based on the published crates.io `noq` 1.3.0 package, registry SHA-256
`b78be567e796cfa74bb9bdc4117790af2d505eb887019cf8803244353eb09d89`.
The package's `.cargo_vcs_info.json` names upstream commit
`c1f411562e6078852749b8bcf1190523096a107f`, path `noq`.

MIOPUNCH-21 changes only `src/connection.rs` and its public re-export in
`src/lib.rs`: a monotonic per-connection path predicate runs under the
connection-driver lock before accepting an inbound datagram and before giving
an outbound packet to the transport. It clears a buffered packet that was
already waiting on a newly forbidden path and counts rejected packets.
This is a packet path gate; it does not inspect or decrypt application frames.
The dependency is selected by the root `[patch.crates-io]` entry.

MIOPUNCH-61 extends the predicate from a remote-only address to a full
`NetworkPath`, preserving the old remote-only wrapper. The original Connection
can therefore reject a private local UDP binding owned by another connection
before receive or transmit processing. `Connection::network_paths` exposes
all protocol-retained paths, including opening and abandonment grace, so the
iroh owner releases a used descriptor only after its final path is discarded.
`NetworkChangeHint::local_ip_for_recovery` lets iroh preserve the real interface
IP of a live binding. MIOPUNCH-90 carries the socket identity separately; the
earlier binding token is no longer used. No packet encoding changes.

MIOPUNCH-47 adds fresh data-path acknowledgment probes. The protocol helper
atomically captures the next data-space packet number and queues an ordinary
PING, then reads the existing largest authenticated per-path ACK watermark.
Closed connections and abandoned paths are rejected. The async wrapper owns
the original Path and opaque watermark, preventing callers from accidentally
checking another connection or path. No QUIC frame encoding, ACK processing,
congestion control or lifetime algorithm changes.

## MIOPUNCH-90: native local socket metadata

`AsyncUdpSocket::poll_recv_with_sockets` reports actual per-entry local socket
identity without changing `RecvMeta` addresses. Existing implementations have
a default method. `UdpSender` adds opt-in support and socket-aware sending;
unsupported explicit bindings fail without default-socket fallback. New
`open_path_on_socket`/`open_path_ensure_on_socket` methods check sender support.
The driver retains `SocketTransmit` identity while sends are pending, including
permission checks. Stateless responses use their actual ingress socket.
`PathProbe` also captures the existing path generation, invalidating an old
probe after migration. Ordinary upstream callers retain their original APIs.
