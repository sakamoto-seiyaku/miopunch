## 1. OpenSpec

- [x] 1.1 Validate `fix-desktop-window-lifecycle` with OpenSpec strict validation.

## 2. Desktop Lifecycle

- [x] 2.1 Add testable close-decision state for explicit quit, tray selection, quit selection, and dialog failure fallback.
- [x] 2.2 Wire Windows Wails options so close events flow through `OnBeforeClose` instead of unconditional hide.
- [x] 2.3 Add Windows-only tray helper with Open and Quit callbacks, plus non-Windows no-op implementation.
- [x] 2.4 Ensure explicit Quit and tray Quit use normal Wails shutdown and preserve existing managed-daemon ownership.
- [x] 2.5 Make the Windows tray context menu reliable for right-click restore and quit.
- [x] 2.6 Restore the Windows GUI from tray click, double-click, and keyboard activation.

## 3. Tests and Build

- [x] 3.1 Add focused Go unit tests for close-decision behavior and managed daemon shutdown ownership.
- [x] 3.2 Run focused Go tests for `cmd/miopunch-desktop` and `internal/desktopbridge`.
- [x] 3.3 Cross-build the Windows desktop session bundle and rebuild Linux/Windows session artifacts without clearing extracted `data/` or `logs/`.
- [x] 3.4 Re-run focused tests, Windows cross-build, and dirty session bundle refresh for the tray context-menu fix.
- [x] 3.5 Re-run focused tests, Windows cross-build, and dirty session bundle refresh for the tray restore fix.
- [x] 3.6 Decode `NOTIFYICON_VERSION_4` tray callback events from `lParam` low word, then rebuild Linux/Windows session artifacts from a clean extracted state.
- [x] 3.7 Route Windows tray right-click through `WM_CONTEXTMENU` only, decode the v4 anchor point, and rebuild the Windows session artifact in place.
