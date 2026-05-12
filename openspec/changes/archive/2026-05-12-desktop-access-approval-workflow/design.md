## Context

The desktop runtime state API already includes `approval_requests`, but the current implementation derives it from running `approve` tasks and does not expose a true pending-request review loop. The existing `approve` task listens on the invite topic and auto-delivers membership after a valid join request is accepted by the task runtime. That is acceptable for CLI compatibility, but it does not match an owner/admin desktop workflow where a human reviews who is joining before granting access.

Constraints:

- LocalAPI remains IPC-only and task-oriented.
- Desktop runtime state remains the authoritative UI feed.
- CLI-compatible invite, join, and approve behavior must keep working.
- Approval state must not leak invite secrets, net secrets, or private keys.

## Goals / Non-Goals

**Goals:**

- Add an explicit-review mode for approval tasks started by desktop.
- Make pending join requests visible as typed `approval_requests`.
- Let desktop approve or reject a request through a LocalAPI task, including
  after daemon restart while the invite is still unexpired.
- Keep decision handling observable through task state, reports, and desktop runtime events.

**Non-Goals:**

- Do not redesign invite code format.
- Do not replace MQTT invite delivery.
- Do not make approval state a desktop-only shadow model.
- Do not require every CLI approval flow to become interactive.

## Decisions

1. **Use explicit-review approval mode instead of changing default approve behavior.**
   Desktop-created approve listeners pass a review flag. In review mode, valid join requests become pending approval records and no membership bundle is delivered until a decision arrives. Default non-review behavior remains compatible with existing CLI flows.

   Alternative considered: make all approve tasks require manual decisions. Rejected because it would break current CLI and lab assumptions.

2. **Represent decisions as task creation, using kind `approve_decision`.**
   A decision task takes `approve_task_id`, `request_msg_id`, and `decision` (`approve` or `reject`). This keeps writes on the existing `POST /api/v0/tasks` surface and avoids a new one-off LocalAPI route.

   Alternative considered: add `/api/v0/approvals/<id>/approve`. Rejected because it creates a second command model next to tasks.

3. **Publish approval state through desktop runtime events.**
   Pending, approved, rejected, and expired requests appear in `approval_requests` and update via `approval_requests.replace`. The desktop UI must not derive pending approvals by parsing task fact strings.

   Alternative considered: require the frontend to inspect approve task facts. Rejected because the desktop runtime contract already exists to prevent stringly-typed UI state.

4. **Persist only the minimum decision state needed for correctness.**
   Pending and decided request records are stored with the invite approval state so duplicate `request_msg_id` handling remains stable. The private invite store also persists the validated join request body, request reply topic, member public keys, and invite broker list needed to rebuild and publish an approval response after daemon restart. Approving consumes one invite use and caches the membership response. Rejecting records a terminal denial and does not consume invite uses.

   This restart material remains internal to the 0600 invite store. Desktop state, SSE events, task reports, and LocalAPI responses must not expose invite secrets, net secrets, private keys, decrypted membership bundles, raw encrypted payloads, or the private restart material.

   Alternative considered: keep pending requests in memory only. Rejected because desktop refresh/restart would lose the review queue and make duplicate request behavior unclear.

## Risks / Trade-offs

- [Risk] Review mode may lengthen join latency. -> Mitigation: surface waiting state in both join and approval task reports.
- [Risk] Decision state can grow during long-running invites. -> Mitigation: keep records bounded by invite expiry and existing invite-store cleanup behavior.
- [Risk] Restart-capable decisions require persisting joiner-provided material. -> Mitigation: store only validated decision material in the private invite store and redact it from all desktop/API surfaces.
- [Risk] Rejection response compatibility needs care. -> Mitigation: model rejection as a terminal encrypted invite response with no membership bundle and test join failure diagnostics.
- [Risk] More runtime state increases UI complexity. -> Mitigation: keep Access workflow tests authoritative and keep the state shape typed.

## Migration Plan

1. Extend approve task args with explicit review mode while preserving current default behavior.
2. Persist pending approval records plus private restart decision material, and publish `approval_requests.replace` events.
3. Add `approve_decision` task handling for approve/reject.
4. Update the desktop bridge/frontend Access flow to start review-mode approve tasks and render pending decisions.
5. Add Go and Playwright coverage before applying the change to mainline.

Rollback:

- Existing non-review approve behavior remains available. If desktop workflow issues appear, the frontend can temporarily fall back to manual approve-code task creation while the backend task contracts remain intact.

## Open Questions

- None for this proposal.
