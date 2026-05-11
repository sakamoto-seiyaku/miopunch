## 1. Specification

- [x] 1.1 Create proposal, design, delta spec, and task checklist for the desktop invite code display regression.

## 2. Test Coverage

- [x] 2.1 Add browser coverage for delayed invite code delivery through a later `GetTask` response.
- [x] 2.2 Add browser coverage for invite code delivery through a runtime task fact event.
- [x] 2.3 Add browser coverage for successful invite completion with no invite code diagnostic.
- [x] 2.4 Add browser coverage for invite code delivery through a final runtime task snapshot.

## 3. Implementation

- [x] 3.1 Update invite code extraction to prefer structured `invite_code` facts while preserving message-prefix compatibility.
- [x] 3.2 Add a bounded post-create wait for invite task output.
- [x] 3.3 Render a visible missing-code diagnostic when an OK invite task completes without code.
- [x] 3.4 Include current task snapshots on task update events and merge them in the desktop runtime event handler.
- [x] 3.5 Document task SSE updates as coalesced state notifications with reliable current task snapshots.

## 4. Validation

- [x] 4.1 Run frontend syntax and Playwright validation.
- [x] 4.2 Run OpenSpec validation for this change.
- [x] 4.3 Run required mainline validation, commit, and build the latest Debian package.
- [x] 4.4 Run focused Go/Playwright regression tests and rebuild current Linux/Windows session bundles.
- [x] 4.5 Add focused task-manager regression tests for current snapshots on stage/fact/diagnosis/done events.
