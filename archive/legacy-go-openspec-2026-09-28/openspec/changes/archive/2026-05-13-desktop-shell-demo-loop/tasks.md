## 1. Shell View State

- [x] 1.1 Add explicit shell view states for idle, listing, connecting, connected, disconnected, and failed.
- [x] 1.2 Preserve selected peer, target, and session values across disconnects and failures.
- [x] 1.3 Default target to `local` and session to `main` when no discovered value is available.

## 2. Discovery And Attach Flow

- [x] 2.1 Wire the discovery action to create an `sh_ls` task for the selected peer.
- [x] 2.2 Render discovered target or session choices when available from task output.
- [x] 2.3 Wire Connect to create `sh_attach` with the selected peer, target, and session.
- [x] 2.4 Wire Disconnect to close the active terminal transport and leave reconnect available.

## 3. Failure Recovery

- [x] 3.1 Show visible, recoverable errors for `sh_ls` task or bridge failures.
- [x] 3.2 Show visible, recoverable errors for `sh_attach` task creation failures.
- [x] 3.3 Show visible, recoverable errors for terminal bridge setup, WebSocket close, and terminal library load failures.
- [x] 3.4 Keep retry controls enabled when recovery is expected.

## 4. Browser Test Support

- [x] 4.1 Extend fake desktop bridge/runtime support for shell discovery results.
- [x] 4.2 Extend fake terminal bridge behavior for connect, disconnect, and retry cases.
- [x] 4.3 Add Playwright coverage for successful list, connect, disconnect, and reconnect loop.
- [x] 4.4 Add Playwright coverage for discovery and attach failure recovery.

## 5. Verification

- [x] 5.1 Run the desktop frontend Playwright suite for the shell demo loop.
- [x] 5.2 Run focused checks for any touched Go bridge code if implementation expands beyond static frontend/test changes.
- [x] 5.3 Run the full `$dev` gate set before any code-affecting implementation enters mainline.
