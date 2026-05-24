## 1. Runtime Authority

- [ ] 1.1 Add `internal/pocv1/runtime` and implement the fixed six-stage wizard model with the `SecureSession -> Shell` ping gate.
- [ ] 1.2 Implement `UserSummary` and `Evidence` as the only default GUI output layers.
- [ ] 1.3 Enforce the fixed 12-value `UserReasonCode` budget for user-facing GUI failures.

## 2. Desktop Integration

- [ ] 2.1 Expose `/api/v1/poc/runtime` and `/api/v1/poc/runtime/events`, and rewire the desktop runtime to consume typed contracts from `02/03/04/05/06` instead of directly composing legacy task internals.
- [ ] 2.2 Reuse existing desktop/localapi/desktopbridge code only as shell/plumbing.
- [ ] 2.3 Keep GUI rendering as the only default presentation layer for peer list, punch evidence, session state, and shell-stage summaries.

## 3. Acceptance

- [ ] 3.1 Add tests for stage progression, the `SecureSession` ping gate, summary/evidence separation, and `UserReasonCode` budget enforcement.
- [ ] 3.2 Add desktop smokes for Linux and Windows covering the full `Network -> Enroll -> Discover -> Punch -> SecureSession(ping ok) -> Shell` path.
