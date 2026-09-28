## Why

Windows currently hides `miopunch-desktop` when the user clicks the window close button, but the app has no visible tray affordance. Users lose control of the resident GUI and GUI-managed daemon, then must use Task Manager before restarting a clean desktop session.

## What Changes

- Add explicit Windows close semantics: close asks whether to keep running in tray or quit.
- Add a Windows tray affordance for restoring the window and fully quitting.
- Ensure full quit stops only the daemon process started and owned by the GUI.
- Keep Linux close-to-exit behavior unchanged for this change.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: Defines Windows close prompt, tray restore/quit behavior, and GUI-owned daemon shutdown semantics.

## Impact

- Affected desktop runtime:
  - Wails Windows window close handling.
  - `miopunch-desktop` explicit quit path.
  - Windows tray integration.
  - GUI-managed same-user session daemon shutdown.
- No LocalAPI, daemon task, frontend bridge schema, or portable data layout changes.
