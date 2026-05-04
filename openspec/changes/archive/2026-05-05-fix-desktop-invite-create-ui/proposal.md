## Why

The desktop Access invite flow can enter a visible busy state after the user clicks Create and then never show either an invite code or a recoverable error. Existing automated coverage exercises the Go LocalAPI and bridge layers, but does not cover the desktop DOM click flow that regressed.

## What Changes

- Make the desktop invite Create action complete predictably by normalizing no-argument task calls before crossing the Wails bridge.
- Add UI-side recovery for bridge calls that fail or never resolve so the button is re-enabled and a visible error is shown.
- Add a browser-level desktop UI smoke test for Access -> Invite -> Create using a fake Wails bridge.
- Wire the smoke test into CI without adding a frontend build pipeline.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: desktop task-starting flows must either render progress/result or show a recoverable error, and invite creation must be covered by automated UI smoke testing.

## Impact

- Affected code: `cmd/miopunch-desktop/frontend/dist/assets/app.js`.
- Affected tests/tooling: new Playwright smoke tests under `cmd/miopunch-desktop/frontend`, plus CI wiring.
- No LocalAPI wire contract change; no daemon task behavior change.
