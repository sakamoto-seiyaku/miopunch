## Context

Current v1 already tracks peer sessions in memory and already has a `ping` /
`hello` gate for entering `Shell`. What it lacks is a runtime-owned background
mechanism that keeps a validated session alive after the initial proof succeeds.

The dataplane idle closer should remain the authority for truly idle sessions.
This change only restores the product behavior that a validated session can stay
reusable across a short quiet period before the next shell action.

## Goals / Non-Goals

**Goals:**
- Keep validated peer sessions alive without changing the shell wire contract.
- Reuse existing `shellproto` ping semantics instead of inventing a new
  keepalive frame.
- Preserve current idle-timeout semantics for unvalidated or dead sessions.
- Add focused runtime tests for keepalive success and failure.

**Non-Goals:**
- Do not change the dataplane default idle timeout.
- Do not add a shell-only idle-timeout bypass.
- Do not redesign `pingGate`, `sh`, or the current shell target protocol.

## Decisions

1. **Place the keepalive loop in `internal/pocv1/runtime`.**
   The current product behavior is owned by the extracted v1 runtime, not by the
   generic dataplane. The runtime already owns `pingGate`, session registration,
   and shell gating, so it is the right layer to decide which sessions deserve
   proactive keepalive.

2. **Keep alive only validated sessions.**
   A session becomes keepalive-eligible only after `pingGate` has been marked
   for that peer via a successful local `ping` or inbound `hello`/`ping`.
   Unvalidated sessions remain subject to normal idle timeout.

3. **Reuse the existing application-level ping path.**
   The keepalive worker opens a normal `shell.v0` stream and performs the same
   `ping` control exchange already used by `miopunch ping`. This keeps the wire
   shape unchanged and refreshes session activity through the existing stream
   read/write accounting.

4. **Close sessions on keepalive failure.**
   If a keepalive attempt cannot open a stream or complete the ping exchange,
   the runtime closes that session as `transport_fatal`. The next foreground
   operation can then establish a fresh session instead of trying to reuse a bad
   one.

## Risks / Trade-offs

- **[Risk]** A validated but otherwise unused session now stays alive longer
  than before.
  **Mitigation:** only sessions with a positive `pingGate` are eligible.

- **[Risk]** A failed keepalive could race with foreground session use.
  **Mitigation:** operate only on currently healthy sessions and let foreground
  `ensurePeerSession()` recreate sessions when a keepalive has already closed
  one.

## Migration Plan

No persisted-state migration is required. The change is an in-memory runtime
worker plus tests.
