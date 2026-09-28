# 2026-06-03 POC v1 Archived UDP Semantics / Owner-Demux Violations

## Purpose

本文记录当前 POC v1 已确认的 UDP 打洞语义偏离和 UDP socket owner / demux
约束违约点。

这不是面试材料，也不是一般实现细节清单。它用于约束后续修复：POC v1
的 UDP 打洞应以已归档且曾经跑通的 UDP 链路为基线，不能在实现里继续通过
raw `*net.UDPConn` 临时接线、重写 assisted candidate 收集、或重写 selected
socket 生命周期来绕过原本已经成立的设计。

## Baseline Rule

当前修复基线已经明确：

```text
POC v1 UDP punching uses the archived working UDP punching chain as the source
of truth.

Allowed removals:
- TCP punching.
- CN-STUN / mainland-vs-global STUN view arbitration.

Everything else in the archived UDP punching semantics must be preserved unless
a new explicit decision records a narrower POC v1 scope.
```

必须保留的 UDP 语义包括：

- ordinary STUN mapped address sampling.
- easy/hard NAT feature analysis.
- UDP mode0-mode4 decision behavior.
- `CandidatePorts`, `SendRandomPorts`, and `ListenRandomPorts`.
- assisted/private candidate exchange.
- selected UDP socket lifecycle.
- traversal and KCP/QUIC dataplane packet demuxing.
- daemon analyzer success feedback.

## Governing Constraint

主线 owner/demux 约束已经明确：

```text
For any UDP-based session establishment, the system SHALL enforce a single UDP
socket owner / demux boundary.
Only the socket owner is allowed to receive UDP packets from the underlying
socket.
```

来源：

- `openspec/specs/miopunch-udp-socket-owner-demux/spec.md`
- `openspec/changes/archive/2026-04-29-udp-socket-owner-demux/design.md`
- `docs/decisions/p3-miopunch-transport-charter.md`

归档版 POC v0 的实际 UDP/KCP 链路也体现了这个边界：

```text
archive/_legacy_poc_v0/internal/task/poc_dial.go
  -> connectivity.Gather
  -> punchdecision.AnalyzeWithDaemonMemory
  -> KCPOwner / TraversalDemux
  -> connectivity.Attempt
  -> DialSessionWithKCPPacketConn when selected conn is runtime-owned
  -> DialSession when selected conn is a temporary winner
```

## Confirmed Violations

### V1: POC v1 punch creates per-run raw UDP traversal demuxes

Current implementation:

```text
internal/pocv1/punch/runtime.go
runPunch(...)
  -> udpowner.NewUDPTraversalDemux(cfg.UDPConn, ...)
```

Impact:

- Every POC v1 punch run can start a new receive loop over the Runtime-owned
  `*net.UDPConn`.
- Runtime inbound accept and outbound dial can therefore create multiple readers
  on the same UDP socket.
- This violates the single owner / demux boundary before secure-session handoff
  even begins.

Required correction:

- Runtime must own or provide a long-lived UDP owner/demux boundary.
- POC v1 punch must consume a traversal endpoint/demux view supplied by Runtime,
  not create a fresh raw `NewUDPTraversalDemux` for every dial/accept run.

### V2: POC v1 secure session upgrades raw UDPConn directly into KCP

Current implementation:

```text
internal/pocv1/session/session.go
dialTransport(...)
  -> netutil.NewKCPConnFromUDP(result.Conn, ...)

acceptTransport(...)
  -> kcp.ServeConn(nil, 10, 3, result.Conn)
```

Impact:

- KCP is handed the raw `*net.UDPConn` directly.
- Late traversal packets and KCP packets are not separated by the owner/demux
  boundary.
- This is the likely failure class behind the current real smoke evidence:
  punching succeeds, KCP accept observes the peer, then TLS handshake fails with
  `io: read/write on closed pipe`.

Observed real-run evidence:

```text
punch run selected: selected_path=punching_ipv4
secure session kcp accept selected
secure session tls handshake failed: err=io: read/write on closed pipe
```

Required correction:

- When selected UDP conn is Runtime-owned, session upgrade must use the
  Runtime-owned KCP PacketConn view from `KCPOwner`, not the raw UDP socket.
- The same owner must route traversal packets away from KCP.

### V3: PathResult exposes raw UDPConn and encodes the wrong ownership model

Current implementation:

```text
internal/pocv1/punch/types.go
type PathResult struct {
    Conn *net.UDPConn
    ...
}

func (r PathResult) Close() error { return nil }
```

Impact:

- `PathResult` makes downstream layers believe every selected UDP path is a
  borrowed Runtime socket.
- That is not the archived behavior. In mode2/mode4, `MakeHole` may return a
  temporary random-listen winner that must be owned by the selected path/session.
- The type exposes raw read authority and makes the session layer bypass
  `KCPOwner` by convenience.

Required correction:

- `PathResult` must carry selected remote endpoint, trusted identity, evidence,
  and explicit ownership/session-handoff information.
- It must distinguish Runtime-owned UDPConn from temporary UDPConn winner.
- Raw `*net.UDPConn` should not be a general downstream read boundary.

### V4: mode2/mode4 random-listen winner lifecycle is lost

Current implementation facts:

```text
internal/punching/punching.go
MakeHole(...)
  -> creates ListenRandomPorts temporary UDP sockets
  -> returns the winning UDPConn and remote UDPAddr
```

Current POC v1 handoff:

```text
internal/pocv1/punch/types.go
PathResult.Close() is no-op
PathResult comment says Conn is borrowed from Runtime UDP owner
```

Impact:

- If the selected winner is a temporary random-listen UDPConn, POC v1 has no
  correct ownership transfer or cleanup path.
- If secure session fails after such a winner, the temporary socket can leak.
- If secure session succeeds, the session ownership is still unclear because the
  type claims the socket is Runtime-owned.

Archived behavior:

- If `attemptRes.Conn == gather.UDP4Conn` or `gather.UDP6Conn`, the archived path
  used owner-backed KCP/QUIC handoff.
- If the winner was not a gather/runtime UDPConn, the archived path used the
  normal session dial path that owns the selected UDPConn.

Required correction:

- Restore this two-case ownership model.
- Tests must cover a random-listen winner and assert it remains usable and is
  eventually owned/closed by the selected path/session.

### V5: Accept-side secure-session failure evidence is under-reported

Current implementation:

```text
internal/pocv1/runtime/runtime.go
sess, err := session.Accept(...)
if err != nil {
    _ = result.Close()
    if r.ctx.Err() != nil { return }
    continue
}
```

Impact:

- The accept side can fail secure-session upgrade without an operator-visible log
  carrying selected path, remote peer, remote UDP, or error.
- This does not cause the socket-owner bug, but it materially increases debug
  time and hides whether the failure is in Punch, KCP, TLS, or yamux.

Required correction:

- Log accept-side secure-session upgrade failures with `remote_peer_id`,
  `selected_path`, `remote_udp`, and error.
- Keep this as diagnostic support, not a substitute for restoring owner/demux
  and selected socket ownership.

### V6: Tests do not cover punching-to-secure-session handoff

Current coverage gap:

```text
internal/pocv1/session/session_test.go
session fixtures use SelectedPath: direct_ipv4
```

Impact:

- `PathResult -> PeerSession` tests prove the direct/simple KCP+TLS+yamux path,
  but do not prove the real `punching_ipv4` path after traversal traffic.
- There is no regression test that sends late tagged traversal packets while
  secure-session upgrade is starting.
- There is no test asserting that POC v1 punch/session cannot create independent
  raw `ReadFromUDP` loops on the Runtime UDP socket.

Required correction:

- Add a focused regression test for `punching_ipv4 -> session.Dial/Accept`.
- Add a test or code-level seam that proves POC v1 punch/session cannot create
  independent raw `ReadFromUDP` loops on the Runtime UDP socket.

### V7: POC v1 hard-codes IPv4 and drops archived UDP6 direct support

Current implementation:

```text
internal/pocv1/punch/snapshot.go
P2PIPFamily: connectivity.P2PIPFamilyV4

internal/pocv1/punch/runtime.go
P2PIPFamily: connectivity.P2PIPFamilyV4
```

Impact:

- POC v1 active punch path cannot gather or attempt UDP6 direct paths.
- This is outside the agreed removal set. TCP punching and CN-STUN special
  handling may be removed, but UDP6 direct support was part of the archived UDP
  path unless a new explicit decision narrows POC v1 to IPv4-only.

Archived behavior:

- The archived path carried `UDP6Conn`, `UDP6TraversalDemux`, and UDP6 owner
  handoff through `connectivity.Attempt`.

Required correction:

- Either restore UDP6 direct support in POC v1, or record an explicit decision
  that POC v1 is IPv4-only.
- Until such a decision exists, this remains a semantic regression against the
  archived UDP baseline.

### V8: Assisted/private candidate collection was rewritten

Current implementation:

```text
internal/pocv1/runtime/runtime.go
localCandidates(...)
  -> collect interfaces
  -> reject docker/br/veth/cni/virbr/vEthernet(Default Switch)

internal/pocv1/punch/snapshot.go
assistedAddrsFromLocalCandidates(...)
```

Impact:

- POC v1 no longer uses the archived gather semantics for assisted/private UDP
  addresses.
- The current implementation can drop addresses that the archived path would
  have exchanged, including virtual or bridge-style addresses that may be useful
  in Windows/WSL/Hyper-V style demos.
- This is a likely contributor to confusing “same host / mirrored network /
  Windows shell” behavior because the runtime now silently classifies local
  interfaces differently from the archived path.

Archived behavior:

```text
connectivity.Gather
  -> nat.ListLocalIPsForNatHole(10)
  -> filters IPv6, loopback, and link-local
  -> emits assisted addrs with the selected UDP port
```

Required correction:

- POC v1 should consume assisted addrs from `connectivity.Gather` or an exact
  equivalent of the archived semantics.
- Do not keep POC v1-specific interface-name filtering unless a new explicit
  decision documents why that behavior is required.

### V9: Daemon analyzer success feedback scope is wrong

Current implementation:

```text
internal/pocv1/punch/exchange.go
answer side computes UDPDecision via AnalyzeWithDaemonMemory(...)

internal/pocv1/punch/runtime.go
both sides call ReportDaemonUDPSuccess(...) with decision.AnalyzerKey
```

Impact:

- The answer side computes the decision using its local `remotePeerID` scope.
- The initiator receives that `AnalyzerKey` and may report success into the
  answer side's analyzer scope rather than its own target-peer scope.
- This does not necessarily break one attempt, but it corrupts or weakens
  daemon-lifetime mode/index scoring.

Archived behavior:

- The same runtime that performed analysis also reported success using its own
  scoped analyzer key.

Required correction:

- Each side must report success into a local analyzer scope that corresponds to
  its own remote peer.
- If the answer carries mode/index for both peers, the analyzer key must either
  be side-local or recomputed locally from the same decision material.

### V10: Tests currently protect wrong assumptions

Current tests:

```text
internal/pocv1/punch/types_test.go
  PathResult.Close() does not close borrowed UDPConn

internal/pocv1/punch/runtime_test.go
  runPunch().Conn must equal original conn

internal/pocv1/runtime/runtime_test.go
  localCandidatesForPort filters virtual/vEthernet-style interfaces
```

Impact:

- These tests lock in assumptions that conflict with the archived UDP baseline.
- They make a correct repair harder because a repair must distinguish
  Runtime-owned conn vs temporary winner conn, and should not silently rewrite
  assisted candidate collection.

Required correction:

- Replace the no-op close test with explicit ownership cases.
- Replace the `runPunch` original-conn assertion with cases for Runtime-owned
  winner and temporary winner.
- Rewrite assisted candidate tests against archived gather semantics, or document
  an explicit POC v1 exception before preserving stricter filtering.

### V11: Direct IPv6 preselects an address but session handoff needs observed endpoint nomination

Current implementation:

```text
connectivity.Attempt
  -> parses peer_direct_addrs
  -> tries direct_ipv6
  -> returns one selected remote UDP endpoint

internal/pocv1/session/session.go
  -> acceptTransport(...)
  -> rejects KCP sessions when actual_remote_addr != expected_remote_addr
```

Impact:

- On hosts with multiple IPv6 addresses on the same interface, the IPv6 address
  used for direct SID probing can differ from the source address selected by the
  OS for the later KCP packets.
- The direct IPv6 probe can therefore prove that the peer is reachable, but the
  secure-session accept side can still reject the KCP session as a remote address
  mismatch.
- This is not a NAT classification problem. IPv6 direct addresses are
  reachability candidates, not the peer identity and not a unique final session
  endpoint.

Observed real-run evidence:

```text
punch run selected: selected_path=direct_ipv6 ownership=runtime
secure session kcp accept rejected remote mismatch:
  expected_remote_addr=[fd76:...:49f3...]:47126
  actual_remote_addr=[fd76:...:d173...]:47126
```

Required correction:

- Treat exchanged IPv6 direct addresses as a bounded candidate set.
- Filter only clearly unusable IPv6 addresses such as loopback, unspecified,
  multicast, and link-local.
- Preserve both global IPv6 and ULA IPv6 candidates within the configured
  direct-candidate cap; do not discard ULA solely because a global candidate
  exists.
- Use SID probing to nominate the observed remote endpoint.
- For `direct_ipv6`, allow secure-session accept to receive KCP from the
  observed endpoint or from the validated peer's exchanged IPv6 direct candidate
  set during the handoff window.
- Keep TLS peer identity validation as the authority for session identity. IP
  address matching is a routing guard, not authentication.

## Not Violations Under Current Baseline

The following are not counted as current POC v1 semantic violations:

- POC v1 active path is `udp_only`; it does not actively select TCP punching.
- `DisableSTUNViewArbitration: true` means built-in STUN is used as one ordinary
  server set. It does not feed CN/global view arbitration into POC v1 decision
  material.
- TCP-related code may remain in shared `connectivity`, `punchdecision`, or
  `wire` packages as long as POC v1 does not activate it.

## Classification

This is primarily an implementation/adherence failure, with an API-contract
weakness.

- The high-level design principle is correct: traversal and dataplane traffic
  must share the same UDP mapping behind one owner/demux.
- The archived UDP path already had a workable handoff model.
- POC v1 did not merely delete TCP and CN-STUN special handling; it also rewrote
  parts of UDP ownership, assisted candidate collection, IPv6 reachability, and
  analyzer feedback.
- Because `PathResult` exposes raw `*net.UDPConn`, the implementation was able to
  take short paths that violated the intended ownership model.

## Detailed Repair Plan

This plan is the implementation baseline for a standalone OpenSpec change after
`fix-pocv1-udp-owner-session-lifecycle` and
`restore-pocv1-udp-xtcp-decision`.

The earlier lifecycle change fixed the immediate "closed UDP file descriptor"
class, but it intentionally did not redesign KCP demuxing. The later XTCP
decision restoration reconnected UDP gather/decision/attempt, but it still left
POC v1 with the wrong selected-socket and session handoff model. The next
change must treat these as one connected repair, not isolated patches.

### 1. Rebuild the selected UDP path contract

Replace the current implicit contract:

```text
PathResult.Conn is always a borrowed Runtime UDP socket.
PathResult.Close is always no-op.
Session can decide how to use PathResult.Conn.
```

with an explicit contract:

```text
PathResult describes the selected UDP path and carries ownership metadata.

Selected conn kinds:
- runtime-owned UDPConn selected by direct path or normal punching.
- temporary UDPConn selected by mode2/mode4 random-listen punching.

Only the selected conn owner may close the selected socket.
Session upgrade receives an owner-safe transport view, not raw read authority.
```

Implementation requirements:

- Introduce an explicit selected UDP ownership enum or equivalent internal type.
- Preserve selected remote UDP endpoint, trusted remote identity, and evidence.
- Make failed handoff cleanup close temporary winners but never close Runtime's
  long-lived UDP socket.
- Remove or strictly quarantine direct session-layer reads from raw Runtime
  `*net.UDPConn`.

Acceptance criteria:

- A test can construct a runtime-owned `PathResult` and prove cleanup does not
  close the Runtime UDP socket.
- A test can construct a temporary-winner `PathResult` and prove cleanup closes
  it when session handoff fails.
- `PathResult` no longer encodes "all selected sockets are borrowed Runtime
  sockets" in comments, tests, or runtime behavior.

### 2. Restore one Runtime UDP owner for traversal and KCP

Runtime-owned UDP paths must use the same owner/demux model as the archived KCP
path:

```text
Runtime UDPConn
  -> KCPOwner
      -> TraversalDemux for tagged punch/direct packets
      -> PacketConn for KCP packets
```

Implementation requirements:

- Runtime creates and owns `KCPOwner` for the daemon UDP socket.
- POC v1 punch receives the Runtime-owned traversal demux/endpoint view instead
  of creating `NewUDPTraversalDemux(cfg.UDPConn, ...)` per run.
- POC v1 secure-session KCP dial/accept receives the owner PacketConn view when
  the selected conn is Runtime-owned.
- `kcp.ServeConn` and KCP dial must not read directly from Runtime's raw
  `*net.UDPConn`.
- Closing a peer session must close yamux/TLS/KCP session state, but must not
  close Runtime's `KCPOwner` unless Runtime itself is shutting down.

Acceptance criteria:

- There is exactly one Runtime-owned `ReadFromUDP` loop for the daemon UDP
  socket.
- Late tagged traversal packets are routed to traversal demux and not delivered
  to KCP.
- KCP packets are delivered through `KCPOwner.PacketConn()`.

### 3. Preserve mode2/mode4 temporary winner ownership

`internal/punching.MakeHole` may return a temporary UDPConn winner created by
`ListenRandomPorts`. That behavior is part of the archived UDP mode2/mode4
semantics.

Implementation requirements:

- Detect whether `connectivity.AttemptResult.Conn` is the Runtime UDPConn or a
  temporary random-listen UDPConn.
- For temporary winners, bypass Runtime `KCPOwner` and transfer the UDPConn to
  session/data plane ownership.
- On secure-session failure, close the temporary winner.
- On secure-session success, the session close path owns and closes the
  temporary winner.
- Non-winning temporary sockets remain closed by `MakeHole` cleanup.

Acceptance criteria:

- A focused test forces a temporary random-listen winner and proves it remains
  open through handoff.
- A failure-path test proves the temporary winner is closed.
- A success-path test proves session close closes the temporary winner.

### 4. Restore archived assisted candidate semantics

POC v1 must not silently replace archived assisted/private candidate semantics
with runtime-specific interface-name filtering.

Implementation requirements:

- Use `connectivity.Gather` assisted addrs directly, or implement an equivalent
  adapter that matches `nat.ListLocalIPsForNatHole(10)` semantics.
- Do not filter `docker`, `br-*`, `veth`, `cni`, `virbr`, or
  `vEthernet (Default Switch)` at the POC v1 runtime layer unless a new explicit
  decision records that exception.
- Keep loopback, IPv6, and link-local filtering consistent with the archived
  UDP gather baseline.

Acceptance criteria:

- Tests compare assisted addr output against archived gather semantics.
- Existing tests that require virtual-interface filtering are removed or
  rewritten to document an explicit exception.
- Windows/WSL-style private candidates are not silently discarded by POC v1
  runtime-only logic.

### 5. Restore UDP6 direct or record an explicit IPv4-only exception

The accepted removal set is TCP punching and CN-STUN arbitration. UDP6 direct
was not included in that removal set.

Implementation requirements:

- Prefer restoring UDP6 direct gather/attempt for POC v1 using the existing
  `connectivity.Gather` and `connectivity.Attempt` UDP6 support.
- Runtime must create/own the UDP6 socket only when local IPv6 is available and
  POC v1 config allows it.
- UDP6 direct must use the same owner/demux boundary as UDP4 for traversal and
  KCP if a UDP6 session is selected.
- If implementation chooses not to restore UDP6 in this change, add a design
  decision that POC v1 is explicitly IPv4-only and update this violation
  classification.

Acceptance criteria:

- Either POC v1 has a UDP6 direct focused test, or there is an explicit
  IPv4-only decision in the OpenSpec change.
- The code no longer hard-codes `P2PIPFamilyV4` without an accompanying decision.

### 6. Fix daemon analyzer feedback scope

Success feedback must be recorded in the local daemon's analyzer scope, not in
the remote peer's decision scope.

Implementation requirements:

- Ensure each side can report mode/index success using a local analyzer key
  scoped to its own remote peer.
- Either carry side-local analyzer metadata in the answer, or recompute the
  local analyzer key from the same UDP decision material before reporting.
- Keep TCP analyzer feedback out of POC v1 active path.

Acceptance criteria:

- A unit test proves initiator success is recorded under initiator local
  `remotePeerID` scope.
- A unit test proves responder success is recorded under responder local
  `remotePeerID` scope.
- Repeated punch attempts can benefit from each side's own daemon memory.

### 7. Replace tests that protect wrong behavior

Tests must stop encoding the implementation shortcuts.

Required test rewrites:

- Replace `PathResult.Close() does not close borrowed UDPConn` with explicit
  runtime-owned and temporary-owned close cases.
- Replace `runPunch().Conn must equal original conn` with selected-owner cases.
- Replace virtual-interface filtering tests with archived assisted gather
  semantics or an explicit exception.
- Add `punching_ipv4 -> KCP/TLS/yamux` handoff coverage.
- Add late tagged traversal packet demux coverage.
- Add no-competing-raw-UDP-reader coverage for POC v1 Runtime.

Acceptance criteria:

- Tests fail against the current shortcut implementation.
- Tests pass only when owner/demux and selected-socket handoff are restored.

### 8. Validation sequence

Do not start with VM/lab gates. This repair needs focused owner/handoff tests
first.

Focused package validation:

```text
go test ./internal/pocv1/punch ./internal/pocv1/session ./internal/pocv1/runtime ./internal/punching ./connectivity -count=1
```

Host validation after focused tests pass:

```text
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
```

Real CLI smoke after host validation:

```text
init-network
invite / join
ls
bidirectional ping
log inspection for selected_path=punching_ipv4 -> secure session success
```

Lab/VM gates are not the first diagnostic tool for this change. They are useful
only after the focused lifecycle and handoff tests prove the core contract.

## Repair Direction

Do not patch only the TLS error, only the session close path, or only logging.
The repair must first restore the archived UDP punching semantics:

```text
Runtime/gather UDP socket(s)
  -> single owner/demux boundary for runtime-owned UDPConn
  -> traversal endpoint/demux for POC v1 punch
  -> connectivity.Attempt direct-first + UDP punching fallback
  -> selected UDP winner classification:
       - runtime-owned winner: KCP uses owner PacketConn
       - temporary random-listen winner: session/data plane owns selected UDPConn
  -> PathResult carries selected endpoint + trusted identity + evidence +
     explicit ownership/session handoff, not raw read authority
  -> local analyzer success feedback uses the local peer scope
```

Repair order should be:

1. Fix `PathResult` / selected socket ownership model.
2. Restore runtime-owned `KCPOwner` handoff for KCP session.
3. Preserve temporary random-listen winner ownership and cleanup.
4. Restore assisted candidate collection to archived gather semantics.
5. Decide explicitly whether UDP6 direct is in POC v1 scope; restore it unless
   an IPv4-only exception is recorded.
6. Fix analyzer success feedback scope.
7. Replace tests that protect the wrong assumptions.
8. Fix direct IPv6 handoff to use observed endpoint nomination instead of
   single-address preselection.
9. Add focused handoff tests before rerunning real CLI smoke.

After code repair, run focused validation first:

- POC v1 punch/session tests.
- `punching_ipv4 -> KCP/TLS/yamux` handoff test.
- late tagged traversal packet does not enter KCP.
- mode2/mode4 temporary winner ownership test.
- assisted candidate collection regression.
- direct IPv6 with multiple peer candidates and a different observed endpoint.

Only after focused validation passes, re-run real CLI smoke:

- init-network
- invite / join
- ls
- ping
- log inspection proving `selected_path=punching_ipv4` reaches secure session
  without `closed pipe` / timeout at TLS
