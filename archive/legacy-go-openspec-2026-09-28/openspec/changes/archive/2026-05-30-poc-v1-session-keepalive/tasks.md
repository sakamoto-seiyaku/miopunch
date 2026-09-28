## 1. Runtime Keepalive

- [x] 1.1 Add a runtime-owned background keepalive loop for validated peer sessions.
- [x] 1.2 Reuse the existing `shellproto` ping exchange so keepalive refreshes session activity without new wire frames.
- [x] 1.3 Close sessions on keepalive failure so the next foreground action can rebuild cleanly.

## 2. Tests

- [x] 2.1 Add runtime tests proving a validated session stays reusable across a quiet period when keepalive is running.
- [x] 2.2 Add runtime tests proving truly idle unvalidated sessions still expire by dataplane idle timeout.
- [x] 2.3 Add runtime tests proving keepalive failure closes the session and forces a fresh rebuild path.

## 3. Verification

- [x] 3.1 Run focused Go tests for `internal/pocv1/runtime`.
- [x] 3.2 Run repo-wide Go formatting and targeted validation for the touched runtime code.
