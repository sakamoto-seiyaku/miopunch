## Context

The desktop GUI already has peer actions for `sh_ls` and `sh_attach`, and the shell protocol is covered by existing LocalAPI and lab specs. The remaining gap is the desktop operator loop: a user should be able to demonstrate shell usability from the GUI without knowing task internals or manually resetting state after disconnects and failures.

Constraints:

- Keep `sh_attach` WebSocket protocol `miopunch.sh.v0` unchanged.
- Keep `sh_ls` and `sh_attach` task kinds unchanged.
- Keep validation at the desktop browser-test layer for this change.
- Do not add a new frontend framework or build pipeline.

## Goals / Non-Goals

**Goals:**

- Make the desktop shell path an obvious loop: select peer, list targets/sessions, connect, disconnect, retry.
- Preserve selected target/session values and visible terminal status.
- Make task, terminal bridge, and WebSocket failures visible and recoverable.
- Add Playwright coverage against the committed static UI using fake bridge/runtime behavior.

**Non-Goals:**

- Do not redesign shell transport, tmux anchoring, locking, or LocalAPI WebSocket semantics.
- Do not add a live lab gate for desktop shell in this proposal.
- Do not replace existing CLI shell behavior.

## Decisions

1. **Treat shell demo loop as desktop UI behavior.**
   The change should improve how the GUI composes existing `sh_ls`, `sh_attach`, terminal bridge, and runtime updates. It should not create new shell task kinds or transport messages.

   Alternative considered: add a dedicated demo backend endpoint. Rejected because it would duplicate the product path that the demo is meant to prove.

2. **Make `sh_ls` the discovery step before attach.**
   The UI should expose target/session discovery and then let the user attach using visible selected values. Defaults remain `local` and `main` when discovery has no richer result.

   Alternative considered: keep Shell as a one-click attach only. Rejected because it hides target/session behavior and makes retries harder to explain.

3. **Use explicit shell connection state in the frontend.**
   The shell panel should distinguish idle, listing, connecting, connected, disconnected, and failed states. Disconnect should close the active terminal bridge/WebSocket path and leave the panel ready to reconnect.

   Alternative considered: infer state only from task cards. Rejected because task state and terminal transport state can diverge during bridge failures.

4. **Validate with browser tests and fake bridge/runtime.**
   The acceptance path should assert DOM behavior, bridge calls, runtime events, and terminal status in Playwright. Existing Go/lab shell coverage continues to validate live transport.

   Alternative considered: add a lab gate for desktop shell now. Rejected because the requested acceptance scope is desktop UI plus browser tests.

## Risks / Trade-offs

- [Risk] Browser tests can overfit fake bridge behavior. -> Mitigation: keep fake responses aligned with existing task and runtime contracts.
- [Risk] UI may show connected while the task has failed. -> Mitigation: track task and terminal transport state separately and render both when they differ.
- [Risk] The committed static asset remains hand-edited. -> Mitigation: keep changes narrow and make Playwright tests the behavioral contract.

## Migration Plan

1. Update the shell panel to show discovery, selected target/session, connection status, disconnect, and retry controls.
2. Wire discovery to `sh_ls` and attach to existing `sh_attach` plus terminal bridge behavior.
3. Add fake bridge/runtime support for shell listing, connect, failure, disconnect, and retry cases.
4. Add Playwright shell demo-loop tests.

Rollback:

- The current peer Shell action can remain as the entry point. If the expanded panel regresses, it can fall back to one-click attach using the existing `sh_attach` behavior.

## Open Questions

- None for this proposal.
