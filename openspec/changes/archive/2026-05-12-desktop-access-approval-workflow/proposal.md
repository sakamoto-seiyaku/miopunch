## Why

The desktop Access tab currently exposes manual invite, join, and approve task entry points, but it does not provide an operator-facing approval workflow for pending join requests. The runtime-state foundation already reserves `approval_requests`; this change turns that reserved surface into an explicit review flow so owner/admin users can approve or reject desktop join requests before membership is delivered.

## What Changes

- Add an explicit-review approval mode for desktop-managed `approve` tasks.
- Represent pending join requests as typed, decision-addressable `approval_requests` in desktop runtime state.
- Add an approval decision task path so LocalAPI clients can accept or reject a pending request through the existing task system.
- Update the desktop Access UI to show pending requests, approve/reject actions, decision progress, and recoverable failures.
- Preserve existing invite/join/approve task compatibility for CLI and non-review flows.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `miopunch-poc-invite-join-approve-v0`: add explicit-review approval semantics and decision handling for join requests.
- `miopunch-poc-localapi-v0`: expose typed approval request runtime state and decision task creation through LocalAPI.
- `miopunch-desktop-gui-v0`: surface pending approval requests in Access and support approve/reject decisions.

## Impact

- Affected code: `internal/task`, `internal/localapi`, `internal/desktopbridge`, and `cmd/miopunch-desktop/frontend/dist`.
- Public behavior: desktop clients receive richer `approval_requests` state and can create approval decision tasks.
- Tests: Go task/LocalAPI tests plus Playwright desktop Access workflow coverage.
- Validation: OpenSpec-only proposal work does not require full `$dev` gates; implementation entering mainline will require focused tests and then the full gate set.
