## 1. Specification

- [x] 1.1 Create proposal, design, delta spec, and task checklist for the desktop invite code display regression.

## 2. Test Coverage

- [x] 2.1 Add browser coverage for delayed invite code delivery through a later `GetTask` response.
- [x] 2.2 Add browser coverage for invite code delivery through a runtime task fact event.
- [x] 2.3 Add browser coverage for successful invite completion with no invite code diagnostic.

## 3. Implementation

- [x] 3.1 Update invite code extraction to prefer structured `invite_code` facts while preserving message-prefix compatibility.
- [x] 3.2 Add a bounded post-create wait for invite task output.
- [x] 3.3 Render a visible missing-code diagnostic when an OK invite task completes without code.

## 4. Validation

- [x] 4.1 Run frontend syntax and Playwright validation.
- [x] 4.2 Run OpenSpec validation for this change.
- [x] 4.3 Run required mainline validation, commit, and build the latest Debian package.
