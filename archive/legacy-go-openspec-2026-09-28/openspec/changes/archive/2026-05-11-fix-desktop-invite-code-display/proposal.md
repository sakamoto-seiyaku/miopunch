## Why

Access -> Invite -> Create can finish successfully without showing the generated invite code, QR code, or enabled Copy action. The current desktop smoke coverage only models an immediate success response, so it misses the real daemon timing where task creation returns before the `invite_code` fact is available.

## What Changes

- Make the desktop invite flow render the generated code when it arrives after task creation through either `GetTask` or runtime task events.
- Preserve invite facts when runtime task events are coalesced by merging the task snapshot carried on the final event.
- Add a visible diagnostic when an invite task completes successfully but no invite code is present in the task output.
- Expand browser tests to cover delayed invite code delivery, event-delivered invite code facts, and missing-code diagnostics.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: desktop invite creation must handle asynchronous task result delivery, not only immediate bridge responses.

## Impact

- Desktop static frontend under `cmd/miopunch-desktop/frontend/dist`.
- Desktop Playwright smoke tests under `cmd/miopunch-desktop/frontend/tests`.
- LocalAPI task event payloads from the daemon now include a current task snapshot so desktop clients can recover facts even when intermediate events are dropped.
