## Why

Current v1 can prove a peer session once with `ping` or `hello`, but the
session may still be reclaimed by the dataplane idle closer before a later `sh`
arrives. That breaks the intended "ping once, reuse later" workflow and forces
an unnecessary re-punch after a short idle gap.

## What Changes

- Add a runtime-owned keepalive loop for validated peer sessions.
- Keep alive only healthy sessions that have already passed the current
  `ping`/`hello` gate.
- Preserve the existing idle timeout and close-reason behavior for truly idle
  sessions.
- Add regression coverage for long gaps between `ping` and later `sh`.

## Capabilities

### New Capabilities
- `miopunch-poc-v1-session-keepalive`: current v1 runtime keeps validated peer
  sessions alive across idle gaps by sending bounded periodic application-level
  keepalive traffic.

### Modified Capabilities
- `miopunch-poc-v1-headless-runtime`: the runtime now owns long-lived session
  reuse for peers that have already passed the `ping`/`hello` gate.

## Impact

- Affected code: `internal/pocv1/runtime`, its tests, and runtime lifecycle
  wiring.
- Behavior: a later `sh` can reuse the existing healthy session after an idle
  gap, instead of forcing a fresh punch when the keepalive loop has kept the
  session active.
