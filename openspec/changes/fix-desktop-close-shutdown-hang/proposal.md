## Why

`miopunch-desktop` can remain running after the user tries to close it. On
Windows, choosing the quit option from the close prompt can leave the desktop
process or GUI-managed daemon alive. On Linux, clicking the window close button
can leave the GUI stuck until the launching terminal sends Ctrl+C.

The likely shutdown blocker is the desktop runtime event pump: shutdown cancels
the pump and waits for its goroutine, but the active LocalAPI event stream body
is not closed, so the reader can remain blocked indefinitely.

## What Changes

- Track and close the active desktop runtime event stream during shutdown.
- Bound the wait for the event pump to stop so close/shutdown cannot hang
  forever.
- Keep Windows close semantics unchanged: tray choice stays resident, quit
  choice exits.
- Keep Linux close semantics as direct quit when there is no reliable tray.
- Preserve daemon ownership boundaries: only GUI-managed session daemons are
  stopped on full quit.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: Clarifies that full desktop shutdown must not be
  blocked by the runtime event stream and must still stop only GUI-managed
  daemons.

## Impact

- Affected desktop runtime:
  - Wails close/shutdown handling.
  - Desktop runtime event stream pump lifecycle.
  - GUI-managed same-user session daemon shutdown ordering.
- No frontend UI, LocalAPI wire contract, tray menu, or daemon ownership
  behavior changes.
