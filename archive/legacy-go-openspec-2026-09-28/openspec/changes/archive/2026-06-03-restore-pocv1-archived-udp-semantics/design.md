## Context

POC v1 has gone through two narrower repairs. `fix-pocv1-udp-owner-session-lifecycle`
stopped the most visible class of bugs where a borrowed Runtime UDP socket was
closed by `PathResult` or session cleanup. `restore-pocv1-udp-xtcp-decision`
reconnected UDP gather, decision, and `connectivity.Attempt`.

Those repairs still leave the active POC v1 path semantically different from
the archived working UDP chain. The implementation still treats `PathResult`
as a raw UDPConn handoff, lets KCP read Runtime's raw UDP socket directly,
does not model temporary random-listen winners as owned selected paths, rewrites
assisted candidate collection, hard-codes IPv4, and can report analyzer success
under the wrong local scope.

The source of truth for this change is
`docs/notes/2026-06-03-pocv1-owner-demux-constraint-violations.md`.

## Goals / Non-Goals

**Goals:**

- Restore archived UDP selected-socket semantics in POC v1.
- Enforce one Runtime-owned UDP owner/demux boundary for runtime-owned UDP paths.
- Preserve mode2/mode4 temporary random-listen winner ownership and cleanup.
- Restore assisted/private candidate semantics from archived gather behavior.
- Restore UDP6 direct support unless an explicit IPv4-only decision is added.
- Ensure daemon analyzer success feedback is scoped to each local peer's
  remote-peer/protocol view.
- Replace tests that currently encode the wrong ownership and candidate
  assumptions.

**Non-Goals:**

- Do not restore TCP punching, TCP direct, relay, or carrier negotiation.
- Do not restore CN/global STUN view arbitration in POC v1.
- Do not change CLI syntax or GUI API shape.
- Do not use VM/lab gates as the first diagnostic step for this repair.

## Decisions

### 1. PathResult becomes an explicit selected-path handoff

`PathResult` will describe the selected UDP path, not expose general raw socket
read authority. It must carry enough information for the next layer to choose
the correct handoff:

- runtime-owned UDP winner: session uses Runtime's owner-backed KCP PacketConn.
- temporary UDP winner: session owns the selected UDPConn and closes it on
  failure or session close.

Alternative considered: keep `PathResult.Conn *net.UDPConn` as the only handoff
and document careful usage. That repeats the failure mode because it lets the
session layer bypass the owner/demux boundary.

### 2. Runtime-owned UDP paths use KCPOwner

Runtime will own the long-lived UDP socket and its owner/demux object. POC v1
punch will use the traversal demux view from that owner. KCP dial/accept will
use the owner's PacketConn view. No POC v1 code path may create a competing
`ReadFromUDP` loop over the Runtime-owned socket.

Alternative considered: create a fresh demux for each punch and let KCP read the
raw socket after punch success. Real smoke already showed this can produce
`punching_ipv4` success followed by secure-session failure.

### 3. Temporary random-listen winners are session-owned

`internal/punching.MakeHole` can return a UDPConn opened for `ListenRandomPorts`.
That socket is not Runtime-owned. On success, the selected path/session owns it.
On secure-session failure, the failed handoff closes it. Non-winning temporary
sockets remain owned and cleaned up by `MakeHole`.

Alternative considered: force all selected paths back to the Runtime UDPConn.
That would remove part of mode2/mode4 behavior and diverge from the archived UDP
algorithm.

### 4. Assisted candidate collection follows archived gather semantics

POC v1 must not add runtime-specific interface-name filtering for assisted UDP
candidates. It should consume `connectivity.Gather` assisted addresses or an
equivalent adapter matching `nat.ListLocalIPsForNatHole(10)` semantics.

Alternative considered: preserve current filtering of Docker, bridge, veth, CNI,
virbr, and Hyper-V default switch names. That may avoid noisy candidates, but it
is not the archived UDP baseline and can hide useful Windows/WSL candidates.

### 5. UDP6 direct is restored unless explicitly scoped out

The accepted removal set is TCP punching and CN-STUN arbitration. UDP6 direct
was not included. This change should restore UDP6 direct through the existing
`connectivity.Gather` / `connectivity.Attempt` support. If implementation proves
that POC v1 must remain IPv4-only, that must be recorded as an explicit design
decision and the specs must be updated accordingly.

Alternative considered: continue hard-coding IPv4 because current demos are
IPv4. That leaves a hidden scope change and conflicts with the archived UDP
baseline.

### 6. Analyzer success feedback is local-scope

Each side must report successful UDP mode/index feedback under its own local
remote-peer/protocol scope. The answer side may still compute the decision, but
the initiator must not write success into the answer side's analyzer key.

Alternative considered: carry one analyzer key in `dial_answer` and let both
peers report with it. That weakens daemon-lifetime learning because the key is
not guaranteed to represent both local peer scopes.

### 7. Direct IPv6 uses observed endpoint nomination

IPv6 direct addresses are reachability candidates. They are not the peer
identity and are not guaranteed to be the exact UDP source address used by later
KCP packets on a multi-address host.

POC v1 will keep the IPv6 path simple:

- gather and exchange bounded IPv6 direct candidates.
- filter only clearly unusable IPv6 addresses: loopback, unspecified,
  multicast, and link-local.
- keep global IPv6 and ULA candidates within the direct-candidate cap.
- fan out SID probing across the exchanged IPv6 direct candidates.
- nominate the observed remote endpoint that answers the SID probe.
- allow secure-session accept for `direct_ipv6` to match either the observed
  endpoint or one of the validated peer's exchanged IPv6 direct candidates.
- rely on TLS peer identity validation for authentication after KCP connects.

Alternative considered: preselect a single preferred IPv6 candidate using
global/ULA and prefix-order rules, then require KCP to use that exact address.
That repeats the observed failure mode because the OS can select a different
source IPv6 address for the same peer and port.

## Risks / Trade-offs

- Raw socket ownership refactor can break working direct paths -> add focused
  direct and punching handoff tests before real CLI smoke.
- Owner PacketConn close semantics can accidentally close Runtime owner -> keep
  Runtime as the owner of owner shutdown and prevent session close from closing
  Runtime-owned owner state.
- Restoring assisted candidates can increase noisy candidate count -> keep
  evidence and bounded attempts; do not silently filter without a decision.
- UDP6 direct may be unavailable on many hosts -> tests should skip unavailable
  local IPv6 rather than treating lack of IPv6 as failure.
- Multi-address IPv6 hosts can produce a different KCP source address than the
  first SID candidate -> direct IPv6 handoff must use observed endpoint
  nomination and bounded candidate matching.
- Analyzer scope fixes may require additional metadata or recomputation -> keep
  the wire shape minimal but make local reporting testable.

## Migration Plan

No persisted state migration is required. Both peers must run the repaired POC
v1 code for the corrected handoff and analyzer behavior.

Rollback is to revert this change and return to the previous UDP-only decision
path. That rollback preserves current CLI syntax but reintroduces the known
owner/demux and selected-socket semantic violations.

## Open Questions

- None for implementation planning. If UDP6 restoration proves infeasible during
  implementation, record an explicit IPv4-only decision before changing tests or
  marking V7 resolved.
