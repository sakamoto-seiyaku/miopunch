## Context

The desktop GUI already enforces single-instance behavior and tracks a `ManagedDaemon` only when the GUI starts the sibling `miopunch up --session` process. `OnShutdown` already stops that managed daemon. The Windows-specific gap is that `HideWindowOnClose=true` hides the Wails window directly, so shutdown never runs and no tray UI is available.

## Goals / Non-Goals

**Goals:**

- Make Windows window-close behavior explicit and recoverable.
- Provide a tray entry when the GUI remains resident.
- Ensure tray Quit and explicit app Quit trigger normal Wails shutdown.
- Stop only GUI-owned session daemons on full quit.

**Non-Goals:**

- Do not add Linux tray support.
- Do not kill external, system, or manually started LocalAPI daemons.
- Do not change LocalAPI or frontend runtime state contracts.

## Decisions

### Use Wails close interception for the prompt

Windows will set `HideWindowOnClose=false` and use `OnBeforeClose` to decide whether to prevent close. The handler shows a native question dialog. Choosing tray hides the window and prevents close. Choosing quit allows close so `OnShutdown` runs.

### Treat explicit Quit as shutdown, not tray minimize

The existing bridge `Quit()` method becomes an explicit quit path by setting an internal flag before calling `runtime.Quit`. The close handler bypasses prompting when that flag is set.

### Implement tray as a Windows-only platform helper

The tray helper owns the Windows tray icon lifecycle and exposes Open/Quit callbacks. Non-Windows builds provide no-op helpers. Tray Open restores the existing Wails window; tray Quit marks explicit quit and calls Wails quit.

The tray helper handles the `WM_CONTEXTMENU` callback used by `NOTIFYICON_VERSION_4` for the tray menu. It decodes the callback event from the low word of `lParam`, because Windows stores the icon identifier in the high word, and it decodes the menu anchor from `wParam`. The menu request is posted back to the tray window before calling `TrackPopupMenu`, avoiding duplicate right-click handling from legacy button-up events. If setting notification icon version 4 fails, the helper falls back to the legacy right-button-up event and cursor position. The menu uses the Win32 foreground-window and returned-command pattern so the user can reliably open or quit from the tray menu.

Tray activation handles both legacy left-click callbacks and newer `NIN_SELECT` / `NIN_KEYSELECT` callbacks through the same decoded event path. Restoring the window shows it, unminimises it, and briefly toggles always-on-top to bring it forward from the tray path.

### Preserve daemon ownership boundaries

The shutdown path keeps using `App.managedDaemon`. If the GUI reused an already-running user/system daemon, `managedDaemon` is nil and no process is stopped.

## Risks / Trade-offs

- Windows tray code touches Win32 message loops. Keep it isolated behind build tags and provide compile-time coverage through Windows cross-build.
- Close prompting cannot be covered by browser tests. Extract the decision behavior into small testable functions and manually verify on Windows.
- If tray initialization fails, choosing tray should fall back to full quit rather than leave a hidden unrecoverable process.
