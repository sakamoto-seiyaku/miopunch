## Context

The desktop shell attach path already sends protocol-level heartbeat frames every
15 seconds in both directions. Even with that traffic, long-lived shell sessions
still die after the dataplane session idle timeout because activity accounting
currently updates only on logical stream open, accept, and close.

This fix spans two layers:
- `dataplane`, where peer-session liveness is tracked
- the desktop shell frontend, where a successful attach still feels stalled
  until the user clicks into the terminal

The change must stay small: no new shell protocol frames, no timeout tuning, no
shell-specific idle-timeout bypass.

## Goals / Non-Goals

**Goals:**
- Make successful logical-stream traffic refresh peer-session activity.
- Keep the existing idle-timeout behavior for truly inactive sessions.
- Ensure an opened desktop shell is ready for immediate keyboard input.
- Add regression coverage for both the session-liveness fix and the focus
  behavior.

**Non-Goals:**
- Do not redesign `miopunch.sh.v0`, heartbeat cadence, or shell lock semantics.
- Do not change the session idle-timeout default or disable it for shell
  traffic.
- Do not expand this into a broader desktop shell UX redesign.

## Decisions

1. **Account for activity at the logical-stream read/write boundary.**
   `logicalStream` is the narrowest shared boundary that sees all application
   traffic, including shell data and heartbeat control frames. Updating session
   activity there fixes the root cause for long-lived streams without special
   casing shell attach.

   Alternative considered: refresh activity in `sh_attach` heartbeat handling
   only. Rejected because it would leave other long-lived logical streams with
   the same bug and would duplicate liveness policy above the transport layer.

2. **Refresh activity only when bytes actually move.**
   Reads and writes that transfer `n > 0` bytes count as activity, including
   partial I/O that returns both data and an error. Zero-byte calls do not count
   as activity.

   Alternative considered: mark every `Read`/`Write` call as activity. Rejected
   because it would blur real traffic with no-op calls and make idle semantics
   less meaningful.

3. **Keep idle closer policy unchanged.**
   The existing idle closer remains the authority for shutting down truly idle
   sessions. This change only improves its signal by feeding it real stream
   traffic.

   Alternative considered: increase `DefaultSessionIdleTimeout`. Rejected
   because it would hide the accounting bug instead of fixing it.

4. **Focus the terminal immediately after `term.open(container)`.**
   This covers both preview and live shell entry points and does not require a
   separate state transition or websocket callback branch.

   Alternative considered: focus only after websocket `onopen`. Rejected
   because preview mode would still require a click and the terminal should be
   ready as soon as it is rendered.

## Risks / Trade-offs

- **[Risk]** Activity accounting on every successful read/write could mask a
  test that implicitly relied on the old bug.
  **Mitigation:** add an explicit regression test that proves truly idle
  sessions still time out.

- **[Risk]** Terminal focus could be flaky in browser tests.
  **Mitigation:** assert against the xterm textarea after the shell reaches the
  connected state instead of assuming synchronous focus before connect settles.

## Migration Plan

No migration or rollout sequencing is required. The change is internal to
session liveness accounting plus a frontend focus polish.

If rollback is needed, revert the logical-stream activity callbacks and terminal
focus call together; no protocol or persisted-state migration is involved.

## Open Questions

- None for this implementation pass.
