## 1. Approval State Model

- [x] 1.1 Extend approve task args with an explicit-review flag while preserving current default behavior.
- [x] 1.2 Add persisted pending approval request records keyed by `approve_task_id` and `request_msg_id`.
- [x] 1.3 Populate non-secret pending request fields from validated join requests.
- [x] 1.4 Coalesce duplicate pending join requests without decrementing invite uses.

## 2. Decision Task Runtime

- [x] 2.1 Add `approve_decision` task arg validation for `approve_task_id`, `request_msg_id`, and `decision`.
- [x] 2.2 Implement approve decisions to publish membership bundles and consume invite uses at most once.
- [x] 2.3 Implement reject decisions to publish terminal rejection responses without consuming invite uses.
- [x] 2.4 Make duplicate and conflicting terminal decisions idempotent and observable.

## 3. LocalAPI And Desktop Runtime

- [x] 3.1 Include typed approval request records in `GET /api/v0/desktop/state`.
- [x] 3.2 Publish `approval_requests.replace` when requests are created, decided, or expired.
- [x] 3.3 Allow `POST /api/v0/tasks` to create `approve_decision` tasks.
- [x] 3.4 Ensure approval request state and reports redact all invite, net, and private key material.

## 4. Desktop Access UI

- [x] 4.1 Start desktop approval listeners in explicit-review mode.
- [x] 4.2 Render pending approval requests in Access for owner/admin users.
- [x] 4.3 Hide approval decision controls for member users.
- [x] 4.4 Submit Approve and Reject actions as `approve_decision` task creations.
- [x] 4.5 Show decision progress, terminal result, and recoverable failure states.

## 5. Verification

- [x] 5.1 Add Go tests for pending request creation, duplicate handling, approve decisions, reject decisions, and conflicting decisions.
- [x] 5.2 Add LocalAPI tests for desktop `approval_requests` state, streamed updates, and `approve_decision` task creation.
- [x] 5.3 Add Playwright tests for pending request rendering, approve/reject actions, role gating, runtime updates, and recoverable decision failures.
- [x] 5.4 Run focused Go and frontend tests for touched packages.
- [x] 5.5 Run the full `$dev` gate set before any code-affecting implementation enters mainline.

## 6. Restart-Durable Decisions

- [x] 6.1 Persist private validated join request decision material and invite brokers for pending approval requests.
- [x] 6.2 Resolve `approve_decision` tasks from persisted approval state when the original approve runtime is no longer active.
- [x] 6.3 Publish approve/reject responses after daemon restart while preserving idempotency and invite use accounting.
- [x] 6.4 Keep persisted decision material redacted from desktop state, SSE events, task reports, and LocalAPI responses.
- [x] 6.5 Add focused Go and LocalAPI coverage for restart decisions and redaction.
- [x] 6.6 Run focused validation for touched packages.
