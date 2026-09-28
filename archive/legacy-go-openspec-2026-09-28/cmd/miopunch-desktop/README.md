# `miopunch-desktop` (Door 1 / D1a)

This binary is the Windows/Linux desktop shell for `miopunch`.

## Frontend assets

The current desktop UI is committed under `cmd/miopunch-desktop/frontend/dist/`
so it can be embedded into the desktop executable without a Node toolchain.

## Developer run path (Linux)

1. Optionally start a daemon for development:

   ```bash
   go run ./cmd/miopunch -- up
   ```

2. Run the desktop app:

   ```bash
   go run -tags desktop,production ./cmd/miopunch-desktop
   ```

If no same-user LocalAPI is already reachable, the release app starts the
sibling `miopunch up` process from the same extracted session bundle.

## Notes

- The real desktop build is behind build tag `desktop`.
- Release desktop builds also require the Wails `production` tag.
- Windows packaging uses Wails manual build tags with WebView2 embed
  (`desktop,production,wv2runtime.embed`).

## Manual smoke checklist (current v1 wizard)

1. Extract the current session bundle as a normal user.
   - Windows: `miopunch_<version>_windows_amd64_session.zip`
   - Linux: `miopunch_<version>_linux_amd64_session.tar.gz`
2. Launch the GUI from a writable extracted directory.
   - Windows: open `miopunch-desktop.exe`.
   - Linux: run `./miopunch-desktop`.
3. Verify local connection.
   - The top-right status chip should show a connected LocalAPI endpoint.
   - The GUI should reuse an existing same-user daemon or start the sibling `miopunch up --session`.
4. Check local logs in the extracted bundle.
   - GUI: `logs/miopunch-desktop.log`
   - Daemon: `logs/miopunch.log`
5. Check local data in the extracted bundle.
   - State: `data/state.json`
   - Derived state: `data/net.json`, `data/identity/`, `data/decls/`,
     `data/bootstrap/`, `data/reports/`
   - Delete `data/` before launch to reset this extracted bundle to a clean node.
6. On Windows, check window and tray lifecycle.
   - Closing the window should ask whether to keep miopunch running in the
     system tray or quit.
   - Choosing the tray option should hide the window while leaving a tray icon.
   - Clicking or double-clicking the tray icon should restore the GUI.
   - Right-clicking the tray icon should keep an `Open miopunch` /
     `Quit miopunch` menu visible until the user chooses an item or dismisses it.
   - `Quit miopunch` should fully exit the GUI and the GUI-managed session
     daemon, but should not stop a daemon the GUI merely reused.
7. On Linux, if startup fails with GTK/display guidance:
   - Run from a local graphical desktop session, not a headless SSH shell.
   - Check `echo "$DISPLAY $WAYLAND_DISPLAY"`.
   - Check `ldd ./miopunch-desktop | grep 'not found'`.
   - Send `logs/miopunch-desktop.log` with the terminal output.
8. Run the Linux two-machine wizard smoke.
   - Machine A: `Network` > bootstrap the current network or create a new one.
   - Machine A: `Enroll` > Create invite, then copy the invite code.
   - Machine A: `Enroll` > Approve a joiner, paste the same code, and keep approval running.
   - Machine B: `Enroll` > Join a network, paste the invite code, and join.
   - Refresh both machines and verify each peer appears in `Discover`.
9. Run the secure-session path.
   - `Punch` or device detail > keep `P2P network` / `IP family` as `auto` for
     the normal smoke, or select a diagnostic override such as `udp_only` / `v4`.
   - Select the remote peer and run Ping.
   - `SecureSession` > confirm the ping gate is satisfied.
   - `Shell` > Find targets or sessions if needed, then Open shell and verify terminal attach.
   - Path policy controls apply only to Ping, Shell target/session discovery,
     and shell attach. They do not change network creation, invite/join,
     roster discovery, MQTT signaling, or STUN discovery.
10. Run the Windows desktop smoke.
   - Verify GUI startup, daemon connection, and runtime contract rendering through the six-stage wizard.
   - Validate summary/evidence rendering and diagnostics export locally.
   - Treat Windows/Linux real-machine interoperability as optional follow-up, not a 07 blocker.
11. Diagnostics
   - Verify the runtime summary stays short while facts/suggestions remain visible under Evidence.
   - Export diagnostics when evidence or logs are needed.

## Packaging notes (session v0)

- Current smoke artifacts are portable session bundles, not privileged installers.
- Each session bundle contains `miopunch-desktop`, sibling `miopunch`, local
  `data/` and `logs/` directories, license/notice files when present, and smoke
  instructions.
- NSIS and `.deb` scaffolds remain in the repo for the deferred `D1a-privileged`
  route. They are not the current wizard smoke gate and must not be required to
  run `install-system-daemon` or `uninstall-system-daemon`.
