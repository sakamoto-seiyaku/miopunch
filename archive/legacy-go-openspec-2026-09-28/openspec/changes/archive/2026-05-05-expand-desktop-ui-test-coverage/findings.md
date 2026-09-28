# Desktop UI Test Findings

This file records product UI behavior issues discovered while expanding browser coverage.

Policy for this change:

- Record product/UI behavior defects here before making product code fixes.
- Do not fix product UI defects by default in this change unless explicitly requested.
- Test harness defects can be fixed directly because they are part of this change.

## Findings

### F-001: Runtime connection events do not re-render the active Diagnostics view

- Source: code inspection while adding `runtime-events.spec.js`.
- Current behavior: the `localapi:connection` runtime handler updates `lastConn` through `renderConnection(conn)`, but does not schedule a render.
- Expected behavior: if the user is already viewing Settings -> Diagnostics, a connection failure event should update the visible suggestions/facts immediately.
- Test status: covered by a passing Playwright regression test.
- Fix status: resolved in this change.

### F-002: Owner Admin deep link can fall back to Network before topology loads

- Source: Playwright coverage for Admin direct navigation.
- Current behavior: opening `/?tab=admin` with an owner bridge can render Network because `initFromQuery()` checks `adminVisible()` before the owner topology snapshot has loaded.
- Expected behavior: an owner/admin deep link to Admin should land on Admin once the snapshot confirms the role, or show a stable loading state until role is known.
- Test status: covered by a passing Playwright regression test.
- Fix status: resolved in this change.
