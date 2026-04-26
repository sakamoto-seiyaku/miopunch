## Context

MNT-01 found several failures while validating real `miopunch up` nodes under MQTT-only signaling. The failures are not independent one-off bugs:

- F-001 shows that MQTT `start_at` exchange readiness is not enough to model NAT role timing for UDP NAT2/NAT3.
- F-002/F-005 show that TCP private listen addresses were modeled as direct candidates instead of assisted punching inputs.
- F-003 shows that the post-punching dataplane exposes a bare stream and conflates an operation stream with the underlying transport session.

The temporary charter at `docs/notes/2026-04-26-punching-phase-scheduler-temporary-charter.md` is the input for this formalization.

## Goals / Non-Goals

**Goals:**

- Promote the temporary design decisions into formal decision docs and OpenSpec deltas.
- Keep signaling, punching decision, attempt execution, and dataplane responsibilities distinct.
- Create a stable design base for three follow-up implementation changes.

**Non-Goals:**

- No Go code changes.
- No lab runner or fixture mutation.
- No old-node wire compatibility work.
- No proactive FRP-style tunnel keepalive implementation.

## Decisions

### 1. Exchange readiness is not punching phase ordering

MQTT and future backends own exchange schedule: delivering snapshots and aligning a start window. They do not own NAT role timing. Receiver low-TTL opening, sender delay, repeated probes, and cancellation belong to punching phase plans produced by decision/executor layers.

### 2. UDP and TCP share the phase scheduler model

UDP and TCP do not share the same probe primitive, but they must share the same mental model:

```text
round snapshot -> phase plan -> receive loop -> delayed/bounded probes -> winner/cancel
```

Protocol-specific details stay in probe adapters.

### 3. TCP assisted candidates are first-class punching inputs

TCP private/local listen addresses are assisted targets, not direct candidates. Direct paths must only consume real direct candidates. Punching may consume assisted exact targets and STUN-derived candidates, but diagnostics must distinguish their source.

### 4. Dataplane exposes sessions and logical streams

Punching success yields a carrier. Dataplane must upgrade that carrier into a secure peer transport session, then expose per-operation generic logical streams. Closing a logical stream must not close the peer session.

## Risks / Trade-offs

- [Risk] Formalization creates overlapping requirements across specs. -> Keep deltas small and put implementation detail into the three follow-up changes.
- [Risk] The session model could over-design future proxy features. -> Only require generic stream kind/metadata now; do not implement front-proxy punching in this change.
- [Risk] TCP assisted fallback could be mistaken for direct success. -> Require target-source diagnostics and keep the successful path name as `punching_tcp4`.
