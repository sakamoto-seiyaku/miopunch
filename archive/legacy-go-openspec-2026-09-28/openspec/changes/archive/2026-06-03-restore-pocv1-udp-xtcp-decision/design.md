## Context

Current POC v1 dial/punch intentionally narrowed `dial_offer` and
`dial_answer` to candidate-only bodies. That preserved roster-backed identity
validation and a clean `PathResult` handoff, but it also removed the archived
UDP NAT/STUN decision material. The runtime now builds local host candidates
from a shared UDP socket, tries direct IPv4, and falls back to a locally
synthesized mode0 `NatHoleResp`.

The archived miopunch path already has the pieces needed for the intended UDP
core: `connectivity.Gather`, `punchdecision.AnalyzeWithDaemonMemory`,
`connectivity.Attempt`, and `internal/punching.MakeHole`. This change reconnects
those pieces for UDP only.

## Goals / Non-Goals

**Goals:**

- Restore UDP-only XTCP-style gather, decision, attempt, and success feedback in
  POC v1.
- Preserve current POC v1 roster authority, `peer_e2e_v1` delivery, runtime UDP
  socket ownership, and `PathResult` output.
- Support ordinary STUN multi-sample mapped addresses from configured servers
  or the archived built-in STUN endpoint set, IPv4 direct candidates, IPv4
  assisted/portmap candidates, and UDP mode0..4 behavior.
- Make mode2/mode4 `ListenRandomPorts` actually run and clean up non-winning
  sockets.

**Non-Goals:**

- Do not restore TCP Door-2, TCP punching, TCP direct, relay fallback, QUIC
  carrier negotiation, or session recipe selection in dial/punch.
- Do not keep the CN/global STUN arbitration path in current POC v1 punch.
- Do not claim IPv6 direct support in this change; IPv6 candidate gathering and
  attempt behavior are follow-up scope.
- Do not reintroduce legacy task, GUI, topology, or session orchestration.

## Decisions

1. **Use the archived UDP decision contract, not FRP's product flow.**

   POC v1 will exchange snapshots equivalent to `NatHoleVisitor` and
   `NatHoleClient`, then use `punchdecision` to derive `NatHoleResp` outputs.
   It will not import FRP naming, work connection, or tunnel-session semantics.

2. **Keep POC v1 identity and messaging ownership.**

   `dial_offer` and `dial_answer` remain peer-E2E messages addressed through the
   trusted roster. The punch snapshot is data inside the existing authenticated
   message, not a separate trust source.

3. **Gather through runtime-owned sockets.**

   The runtime still owns the long-lived UDP socket. Gather must be able to
   observe direct/STUN/portmap state for that socket without rebinding it or
   closing it on success. Any temporary sockets opened for mode2/mode4 are owned
   by the selected path result or closed as non-winners.

4. **Simplify STUN policy for the restored POC v1 path.**

   The restored POC v1 path uses ordinary STUN samples. If joined bootstrap
   state provides explicit STUN servers, it uses those. Otherwise it keeps the
   archived built-in STUN endpoint set as one ordinary sample set. Existing
   CN/global logic can remain elsewhere for legacy compatibility, but POC v1
   exchange must not rely on region-specific STUN arbitration.

5. **Use `connectivity.Attempt` for path order and evidence.**

   POC v1 will pass decision responses to `connectivity.Attempt` with
   `P2PNetworkUDPOnly`. That restores direct-first ordering and keeps the
   punching fallback behavior in one place.

## Risks / Trade-offs

- Random UDP listening can consume local ports and goroutines -> bound requested
  counts, inherit attempt context cancellation, and close every non-winning
  socket.
- Direct paths may hide punching regressions -> keep focused fallback and mode
  tests that force punching.
- Extending `dial_offer` / `dial_answer` changes the current wire body -> keep
  decode validation explicit and update all codec tests.
- Reusing `connectivity.Gather` risks pulling TCP back in -> add UDP-only config
  and tests that assert no TCP behavior in this path.

## Migration Plan

No persisted state migration is required. Both peers must run the restored POC
v1 code for the extended dial bodies. Rollback is to return to the previous
candidate-only direct-first implementation.
