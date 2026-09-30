# Local traversal TTL patch

Base: crates.io `netwatch` 0.19.3, the version already selected by the root lockfile; upstream `n0-computer/net-tools` commit `0030e57fc70981895bb86212ff0f059cc41c8a4e`, directory `netwatch` (published `.cargo_vcs_info.json`). Upstream MIT/Apache licenses and source are retained. No network monitor, rebinding, or QUIC send behavior is changed.

`UdpSocket::poll_send_datagram` sends a bounded raw traversal packet on the existing socket. Linux/Android use one `IP_TTL` control message in `sendmsg`; this never calls `setsockopt` or changes the socket default. Unsupported metadata falls back to a default-TTL raw `SockRef::send_to`. Other platforms use that default path and report degradation. Tokio readiness is managed by the outer `try_io`, with raw OS I/O inside the closure.

The optional `miopunch-test-hooks` feature exposes a per-socket unsupported-TTL fixture. It is forwarded only by iroh's test hook feature and the root NAT lab feature. Production builds have no such override. The hook affects raw traversal metadata; QUIC/QAD senders remain untouched.

The added implementation is `src/udp.rs`; the only manifest change is the optional feature. Real Linux packet TTL, original source ports, default responses, QAD/QUIC isolation, and original authenticated connections are checked in `docs/evidence/mio63-legacy-scheduler/`. Other platforms currently receive the explicit default-TTL best-effort behavior and have no platform-specific packet proof in this slice.
