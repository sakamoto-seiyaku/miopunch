## 1. Punching Wire Tag (PunchTagV1)

- [x] 1.1 Add PunchTagV1 constants and helpers in `internal/punching` (wrap/unwrap: `tag || crypto(payload)`).
- [x] 1.2 Update traversal send paths to prefix tag (direct handshake `NatHoleSid`, punching probes / responses).
- [x] 1.3 Update traversal receive paths to require tag before attempting decrypt/parse.
- [x] 1.4 Add unit tests covering: tag present/absent, wrong version, and non-punching packets are ignored (no decrypt attempts).

## 2. QUIC Socket Owner via `quic.Transport`

- [x] 2.1 Refactor dataplane QUIC listen/dial to use `quic.Transport{Conn: udpConn}` (no `quic.Listen` / `quic.Dial` helpers on the raw conn).
- [x] 2.2 Move traversal receive for QUIC mode to `Transport.ReadNonQUICPacket` (bridge into punching transaction handler).
- [x] 2.3 Move traversal send for QUIC mode to Transport write path (no direct `udpConn.WriteTo` after ownership transfer).
- [x] 2.4 Add regression checks to ensure `--quic-cc bbr|brutal` selection remains effective under the Transport-based setup.

## 3. KCP Socket Owner / Demux

- [x] 3.1 Implement a UDP socket owner loop for KCP mode: single goroutine reads from UDPConn and demuxes by PunchTagV1.
- [x] 3.2 Implement a `net.PacketConn` wrapper backed by the owner queue, exposing only non-punching packets to kcp-go.
- [x] 3.3 Update dataplane KCP listener/dial to consume the wrapper PacketConn and support multi-peer concurrent accept on one UDP port.
- [x] 3.4 Add unit tests for KCP demux: punching packets never reach kcp-go accept path; two distinct remotes can be accepted sequentially without restarting the listener.

## 4. Attempt / Punching I/O Boundary Refactor

- [x] 4.1 Introduce a small “demux endpoint” interface for traversal I/O (recv/send) and wire it into `connectivity.Attempt` and `internal/punching.MakeHole`.
- [x] 4.2 Remove direct `ReadFromUDP` races in attempt/punching by routing all traversal receives through the socket owner (QUIC: ReadNonQUICPacket; KCP: owner queue).
- [x] 4.3 Revisit `attemptMu` serialization in `internal/pocacceptor`: allow concurrency once I/O is correctly demuxed (must keep correctness and deterministic shutdown).

## 5. Acceptor / Listener Integration (Multi-Peer)

- [x] 5.1 Refactor acceptor/server path to use a UDP session listener that can accept multiple peer sessions (QUIC + KCP) on the same port.
- [x] 5.2 Ensure a long-lived session does not block new inbound sessions; verify session lifecycle (close reasons, idle timeout) remains diagnosable.

## 6. Regression Coverage

- [x] 6.1 Add or extend lab/MNT coverage: one acceptor port, at least two peers connect concurrently, each exchanges payload evidence (cover both QUIC and KCP).
- [x] 6.2 Add focused unit tests for demux behavior and shutdown (context cancel closes owners/listeners deterministically).
