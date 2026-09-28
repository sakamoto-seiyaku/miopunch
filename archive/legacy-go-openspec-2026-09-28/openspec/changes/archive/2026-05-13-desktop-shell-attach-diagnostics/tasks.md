## 1. Shell Failure Attribution

- [x] 1.1 Audit the late `sh_attach` failure exits in `internal/task`, `internal/pocacceptor`, and `internal/shelltarget` and choose the final `stage`, `reason_code`, and fact shape for post-attach failures.
- [x] 1.2 Preserve peer, target, session, and failing shell-layer facts in final `sh_attach` task/report output when attach setup has already started.
- [x] 1.3 Ensure abnormal remote shell termination after attach setup no longer collapses to bare `EOF` when richer shell diagnosis is available.

## 2. Attach-Path Logging

- [x] 2.1 Add intent-based `debug` / `info` / `warn` / `error` logging around the shell attach lifecycle in the local task bridge, remote acceptor, and shell target backend.
- [x] 2.2 Use the existing `task_id` plus peer/target/session fields as the shared correlation context across desktop logs, daemon logs, and task diagnostics.
- [x] 2.3 Keep raw transport-close evidence as supplemental facts/log fields when exact backend diagnosis is unavailable.

## 3. Desktop Diagnostic Surface

- [x] 3.1 Update the desktop shell bridge to distinguish explicit disconnect from abnormal shell closure.
- [x] 3.2 Update the desktop shell UI to render a concise failure summary from final `sh_attach` diagnostics while preserving retry for the same peer, target, and session.
- [x] 3.3 Keep a bounded fallback disconnect message for cases where richer shell diagnostics are still unavailable.

## 4. Focused Verification

- [x] 4.1 Add or update Go tests that cover late `sh_attach` failure attribution and final actionable task output.
- [x] 4.2 Add or update desktop/frontend tests that cover abnormal shell close diagnostics and retry preservation.
- [x] 4.3 Re-run the Linux desktop -> Windows shell attach scenario and confirm the resulting logs/task output identify the last known failing shell layer instead of ending only as WebSocket `1006`.

## 5. Final Validation

- [x] 5.1 Run focused verification for the touched frontend and Go packages while implementing the change.
- [x] 5.2 Run the full `$dev` gate set before any code-affecting implementation enters mainline.
