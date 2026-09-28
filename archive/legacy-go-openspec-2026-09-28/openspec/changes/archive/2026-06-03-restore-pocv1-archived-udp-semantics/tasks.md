## 1. Contract Audit And Failing Tests

- [x] 1.1 Add focused tests that fail against the current shortcut implementation for Runtime-owned `PathResult` cleanup, temporary-winner cleanup, and wrong original-conn-only assumptions.
- [x] 1.2 Add focused tests proving POC v1 session code does not read Runtime raw `*net.UDPConn` directly for Runtime-owned selected paths.
- [x] 1.3 Add a focused `punching_ipv4 -> KCP/TLS/yamux` handoff test that injects or preserves late tagged traversal traffic while secure-session setup begins.
- [x] 1.4 Add a mode2/mode4 temporary random-listen winner test that proves the temporary winner stays open through handoff and is closed on failed handoff.
- [x] 1.5 Replace assisted candidate tests that require POC v1 interface-name filtering with tests matching archived `connectivity.Gather` / `nat.ListLocalIPsForNatHole` semantics.
- [x] 1.6 Add analyzer feedback tests proving initiator and responder each report UDP success under their own local remote-peer/protocol scope.

## 2. PathResult And Selected UDP Ownership

- [x] 2.1 Replace the implicit `PathResult.Conn is borrowed Runtime UDPConn` contract with explicit selected UDP ownership metadata.
- [x] 2.2 Represent at least Runtime-owned selected UDP paths and temporary selected UDP paths in the `PathResult` handoff.
- [x] 2.3 Make `PathResult` cleanup close temporary selected UDP sockets on failed handoff while never closing Runtime-owned UDP sockets.
- [x] 2.4 Preserve selected remote UDP endpoint, trusted remote identity, selected path kind, and existing evidence in the new handoff.
- [x] 2.5 Update POC v1 punch tests and helpers to construct both Runtime-owned and temporary-owned selected path results.

## 3. Runtime UDP Owner / Demux

- [x] 3.1 Add Runtime-owned UDP owner state for the daemon UDP socket and define its shutdown lifecycle.
- [x] 3.2 Route POC v1 traversal through the Runtime owner traversal demux instead of creating per-run raw `NewUDPTraversalDemux` readers over the Runtime UDP socket.
- [x] 3.3 Expose an owner-safe KCP PacketConn view for Runtime-owned selected UDP paths.
- [x] 3.4 Ensure peer session close and failed secure-session handoff do not close the Runtime UDP owner or Runtime UDP socket.
- [x] 3.5 Add or update diagnostics that identify Runtime-owned versus temporary selected UDP paths.

## 4. Secure Session Handoff

- [x] 4.1 Update `session.Dial` and `session.Accept` to consume the selected UDP ownership metadata from `PathResult`.
- [x] 4.2 For Runtime-owned selected paths, build KCP dial/accept on the Runtime owner PacketConn view instead of raw UDPConn.
- [x] 4.3 For temporary selected paths, let secure-session handoff own the selected UDPConn and close it on failure.
- [x] 4.4 On successful temporary selected paths, make the resulting peer session close the selected UDPConn when the session closes.
- [x] 4.5 Log and expose secure-session accept failures with remote peer, selected path, remote UDP endpoint, ownership kind, and error.

## 5. Archived UDP Gather Semantics

- [x] 5.1 Remove or bypass POC v1-only assisted candidate collection that filters by interface names such as Docker, bridge, veth, CNI, virbr, or Hyper-V default switch.
- [x] 5.2 Make POC v1 UDP snapshot gathering consume `connectivity.Gather` assisted addresses or an equivalent adapter matching archived gather semantics.
- [x] 5.3 Preserve ordinary built-in STUN as one ordinary server set and keep CN/global STUN arbitration inactive for POC v1.
- [x] 5.4 Ensure UDP snapshot exchange still carries direct, mapped, assisted, credential, and decision material inside the two-message `dial_offer` / `dial_answer` flow.

## 6. UDP6 Direct Support

- [x] 6.1 Remove unconditional POC v1 `P2PIPFamilyV4` hard-coding from UDP snapshot gathering and UDP attempt.
- [x] 6.2 Wire UDP6 direct candidates through POC v1 gather, exchange, attempt, evidence, and session path facts when local UDP6 is available.
- [x] 6.3 Ensure UDP6 direct path evidence reports `selected_path=direct_ipv6`.
- [x] 6.4 If UDP6 cannot be restored in this change, add an explicit IPv4-only decision to the design/specs before marking this task complete.
  - N/A: UDP6 direct was restored and validated by tests plus CLI smoke selecting `direct_ipv6`.

## 7. Analyzer Success Feedback

- [x] 7.1 Change UDP analyzer metadata so each peer can derive or receive a local analyzer key scoped to its own remote peer and protocol.
- [x] 7.2 Report `punching_ipv4` success using the local peer's analyzer key, mode, and index on both initiator and responder.
- [x] 7.3 Keep TCP analyzer feedback inactive in POC v1 UDP-only path.
- [x] 7.4 Update diagnostics or tests so repeated attempts can verify local success memory is reusable.

## 8. Focused Validation

- [x] 8.1 Run `go test ./internal/pocv1/punch ./internal/pocv1/session ./internal/pocv1/runtime ./internal/punching ./connectivity -count=1`.
- [x] 8.2 Run targeted tests for `punching_ipv4 -> KCP/TLS/yamux`, late tagged traversal demux, temporary winner ownership, assisted candidates, UDP6 direct, and analyzer scope.
- [x] 8.3 Run `go test ./...`.
- [x] 8.4 Run `go vet ./...`.
- [x] 8.5 Run `bash scripts/check_no_xtcp_imports.sh`.

## 9. Real CLI Smoke

- [x] 9.1 Rebuild the latest binaries after focused host validation passes.
- [x] 9.2 Run real CLI smoke for `init-network`, `invite`, `join`, `approve`, `ls`, and bidirectional `ping`.
  - 2026-06-03 isolated public-broker session smoke passed from rebuilt bundle: `init-network`, `invite`, approve-before-join, `join`, `ls`, and bidirectional `ping -u` all completed; both pings selected `direct_ipv6`.
  - 2026-06-03 rebuilt bundle again into `/tmp/miopunch-user-request-smoke-20260603T143551`, clean-extracted two nodes, removed each node's `data` and `logs`, started two isolated debug daemons with separate LocalAPI sockets, completed `init-network`, approve-before-join, `join`, `ls`, and bidirectional `ping -u`; both pings returned `ping=ok` and `selected_path=direct_ipv6`.
- [x] 9.3 Inspect logs and confirm selected UDP paths reach secure session without TLS closed-pipe or KCP timeout failure.
  - 2026-06-03 local CLI smoke passed, but this host selected `selected_path=direct_ipv6` with `ownership=runtime`; it did not exercise a real CLI `punching_ipv4` selected path.
  - 2026-06-03 repeated clean two-node host smoke after the direct IPv6 observed-endpoint repair; this host still selected `direct_ipv6` before punching fallback, so `punching_ipv4` remains unexercised by the default same-host smoke.
  - 2026-06-03 Android/Linux real-device validation accepted this item for archive: Android-to-Linux `ping` and `sh ls` completed with selected UDP path evidence, and Linux-to-Android `ping -4` completed with selected UDP path evidence.
  - Focused owner/handoff tests cover the `punching_ipv4 -> KCP/TLS/yamux` and late tagged traversal demux cases; no separate forced real-smoke `punching_ipv4` path was obtained before archive.
- [x] 9.4 Record any lab/VM gate that is intentionally skipped or blocked; do not use lab failure as a substitute for focused owner/handoff tests.
  - 2026-06-03 lab/VM gates were not run in this pass. Focused owner/handoff tests, full Go host gates, and local CLI smoke were run instead.

## 10. Direct IPv6 Observed Endpoint Nomination

- [x] 10.1 Update docs/specs to require direct IPv6 candidate fanout and observed endpoint nomination instead of single-address preselection.
- [x] 10.2 Preserve global IPv6 and ULA IPv6 candidates within the bounded direct-candidate cap.
- [x] 10.3 Carry direct IPv6 allowed remote endpoints through `PathResult`.
- [x] 10.4 Accept direct IPv6 KCP sessions from observed or exchanged peer IPv6 endpoints while still rejecting unknown endpoints.
- [x] 10.5 Add focused tests for IPv6 candidate preservation and direct IPv6 secure-session endpoint matching.
- [x] 10.6 Run focused validation and update this task list with the results.
  - 2026-06-03 passed `go test ./connectivity ./internal/pocv1/punch ./internal/pocv1/session -count=1`.
  - 2026-06-03 passed `go test ./internal/pocv1/punch ./internal/pocv1/session ./internal/pocv1/runtime ./internal/punching ./connectivity -count=1`.
  - 2026-06-03 passed `openspec validate restore-pocv1-archived-udp-semantics --strict`.
  - 2026-06-03 passed `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
  - 2026-06-03 isolated CLI smoke passed with `selected_path=direct_ipv6`; logs showed bounded `allowed_remote_udp_addrs` containing both observed and alternate `fd76` IPv6 endpoints, with no KCP remote mismatch rejection.
