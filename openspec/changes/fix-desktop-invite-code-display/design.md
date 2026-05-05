## Context

`miopunch-desktop` embeds committed static assets and talks to the daemon through Wails bridge methods. `CreateTask("invite", {})` returns as soon as the task is created, while the real invite code is appended later as a task fact. The existing browser test fixture returns a completed task with `invite_code` immediately, so it does not model the daemon timing seen in the installed package.

## Goals / Non-Goals

**Goals:**

- Keep the Access invite panel responsive while the invite task is still producing output.
- Render the code from the existing task fact contract once it arrives through either polling or runtime events.
- Add regression tests for delayed output and missing-code completion.

**Non-Goals:**

- Do not change invite code encoding, LocalAPI task schema, or daemon task execution.
- Do not parse placeholder suggestions such as `miopunch join <invite_code>` as real codes.
- Do not expose invite codes through topology diagnostics or redacted reports.

## Decisions

- Use a short UI polling loop after invite task creation. Runtime SSE events are best-effort from the desktop bridge; polling `GetTask` makes the visible result deterministic even if the event pump starts late or reconnects.
- Prefer structured facts with `term_id: "invite_code"` before falling back to the legacy `message` string prefix. This keeps compatibility with existing task JSON while reducing accidental parsing.
- Treat `done/OK` with no invite code as an explicit UI diagnostic. A silent empty readonly field looks like a broken button and hides an invalid backend/task state from the user.
- Keep the QR renderer tied to the rendered code. If no code is available, the QR area remains empty and Copy stays disabled.

## Risks / Trade-offs

- Polling adds a small number of bridge calls after invite creation. Mitigation: stop as soon as the code appears, task fails, or the wait budget expires.
- A real backend regression could still omit `invite_code`. Mitigation: the UI now surfaces that condition and tests cover it.
- Existing tests may still overuse fake immediate success. Mitigation: add dedicated delayed and event-driven cases while keeping the existing smoke path.
