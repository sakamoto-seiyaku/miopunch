# `miopunch-desktop` (Door 1 / D1a)

This binary is the Windows/Linux desktop shell for `miopunch`.

## Frontend assets

For `v0`, the desktop UI is a fork of `internal/http_panel/assets/` (MD3 + xterm.js),
copied into `cmd/miopunch-desktop/frontend/dist/` so it can be embedded into the
desktop executable without a Node toolchain.

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

## Manual smoke checklist (session v0)

1. Extract the current session bundle as a normal user.
   - Windows: `miopunch_<version>_windows_amd64_session.zip`
   - Linux: `miopunch_<version>_linux_amd64_session.tar.gz`
2. Launch the GUI from a writable extracted directory.
   - Windows: open `miopunch-desktop.exe`.
   - Linux: run `./miopunch-desktop`.
3. Verify local connection.
   - Settings > Diagnostics should show a connected LocalAPI endpoint.
   - The GUI should reuse an existing same-user daemon or start the sibling `miopunch up --session`.
4. Check local logs in the extracted bundle.
   - GUI: `logs/miopunch-desktop.log`
   - Daemon: `logs/miopunch.log`
5. Check local data in the extracted bundle.
   - State: `data/state.json`
   - Derived state: `data/net.json`, `data/identity/`, `data/decls/`,
     `data/bootstrap/`, `data/reports/`
   - Delete `data/` before launch to reset this extracted bundle to a clean node.
6. On Linux, if startup fails with GTK/display guidance:
   - Run from a local graphical desktop session, not a headless SSH shell.
   - Check `echo "$DISPLAY $WAYLAND_DISPLAY"`.
   - Check `ldd ./miopunch-desktop | grep 'not found'`.
   - Send `logs/miopunch-desktop.log` with the terminal output.
7. Run a two-machine access smoke.
   - Machine A: Access > Create invite > Create, then copy the invite code.
   - Machine A: Access > Approve request, paste the code, and start approval.
   - Machine B: Access > Join network, paste the code, and join.
   - Refresh both machines and verify each peer appears in Network.
8. Run peer operations.
   - Open the remote peer and run Ping; expect payload exchange.
   - Run List sessions, then Shell > Connect; verify terminal attach.
9. Reports
   - Open completed task reports and export them when diagnostics are needed.

## Packaging notes (session v0)

- Current smoke artifacts are portable session bundles, not privileged installers.
- Each session bundle contains `miopunch-desktop`, sibling `miopunch`, local
  `data/` and `logs/` directories, license/notice files when present, and smoke
  instructions.
- NSIS and `.deb` scaffolds remain in the repo for the deferred `D1a-privileged`
  route. They are not the current session smoke gate and must not be required to
  run `install-system-daemon` or `uninstall-system-daemon`.
