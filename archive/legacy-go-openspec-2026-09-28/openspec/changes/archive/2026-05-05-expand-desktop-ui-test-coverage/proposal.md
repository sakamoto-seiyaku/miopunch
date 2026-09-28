## Why

The desktop UI now has a real design direction, but automated coverage only exercises the Access invite Create regression. Other tabs and actions can still regress silently because no browser test verifies the desktop interaction model end to end.

## What Changes

- Expand desktop UI tests from one invite smoke path to a CI-run Playwright suite that covers Network, Access, Admin, Settings, runtime events, permissions, disabled states, and recoverable bridge errors.
- Add reusable Playwright fixtures for fake Wails bridge responses so tests exercise the real committed DOM assets without requiring a daemon or a Wails shell.
- Add a dedicated findings log for UI behavior defects discovered by the expanded suite; record product issues there first instead of fixing them automatically.
- Keep the desktop frontend static asset model unchanged and do not introduce a frontend build pipeline.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: desktop GUI behavior that is exposed through committed static assets must have automated browser coverage for primary navigation, core flows, role-gated controls, runtime updates, and recoverable UI failure states.

## Impact

- Affected tests/tooling: Playwright tests under `cmd/miopunch-desktop/frontend`, scoped npm test dependencies, and desktop UI CI execution.
- Affected docs/specs: OpenSpec change artifacts and `findings.md` for defects found during coverage expansion.
- Affected product code: no planned product code changes unless explicitly required; test-discovered UI defects are recorded first.
