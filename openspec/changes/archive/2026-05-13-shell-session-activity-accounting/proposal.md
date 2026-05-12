## Why

Desktop shell attach can now reach a real interactive `ready` state, but long
lived shell sessions still die after roughly two minutes with
`idle_timeout`. The current shell heartbeat is already present; the real bug is
that peer-session activity accounting only updates on stream open/accept/close,
so sustained logical-stream traffic is misclassified as idle.

## What Changes

- Update dataplane session liveness accounting so logical-stream reads and
  writes refresh peer-session activity.
- Preserve the existing idle timeout behavior for truly inactive sessions; do
  not disable idle timeout or add a shell-only bypass.
- Make the desktop embedded terminal claim focus when it opens so a successful
  attach is immediately ready for typing.
- Add regression coverage for long-lived logical-stream traffic, true idle
  timeout, and shell terminal focus on connect.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `miopunch-dataplane`: peer-session idle accounting changes so active logical
  streams keep their transport session alive.
- `miopunch-desktop-gui-v0`: the embedded shell terminal focuses the terminal
  input when a shell session opens.

## Impact

- Affected code: `dataplane`, desktop frontend shell view, and their tests.
- Public APIs/protocols: no new API, task kind, or shell protocol frame.
- Behavior: active shell traffic and shell heartbeat no longer trigger false
  session idle timeouts; successful shell connect becomes immediately ready for
  keyboard input.
