## 1. OpenSpec And UI Fix

- [x] 1.1 Create proposal, design, and delta spec artifacts for the desktop invite Create regression.
- [x] 1.2 Normalize desktop task args so no-argument task actions cross the Wails bridge as an object.
- [x] 1.3 Add UI-side timeout/error recovery for invite creation so the busy state always clears.

## 2. Browser Smoke Coverage

- [x] 2.1 Add a scoped Playwright package under `cmd/miopunch-desktop/frontend`.
- [x] 2.2 Add Access -> Invite -> Create success smoke coverage using a fake Wails bridge.
- [x] 2.3 Add bridge failure or timeout smoke coverage that asserts visible recovery.

## 3. CI And Validation

- [x] 3.1 Wire the desktop UI smoke test into CI.
- [x] 3.2 Run focused validation for the desktop frontend and related Go packages.
- [x] 3.3 Rebuild the Debian package for manual install verification.
