# `miopunch-desktop` (Door 1 / D1a)

This binary is the Windows/Linux desktop shell for `miopunch`.

## Frontend assets

For `v0`, the desktop UI is a fork of `internal/http_panel/assets/` (MD3 + xterm.js),
copied into `cmd/miopunch-desktop/frontend/dist/` so it can be embedded into the
desktop executable without a Node toolchain.

## Developer run path (Linux)

1. Start the daemon (LocalAPI):

   ```bash
   go run ./cmd/miopunch -- up
   ```

2. Run the desktop app:

   ```bash
   go run -tags desktop,production ./cmd/miopunch-desktop
   ```

## Notes

- The real desktop build is behind build tag `desktop`.
- Release desktop builds also require the Wails `production` tag.
- Windows packaging uses Wails manual build tags with WebView2 embed
  (`desktop,production,wv2runtime.embed`).

## Manual smoke checklist (v0)

1. Install + start daemon
   - Windows: run the NSIS installer as Administrator (it calls `miopunch install-system-daemon`).
   - Linux: install the `.deb` as root (postinst calls `miopunch install-system-daemon`).
2. Launch the GUI
   - Open `miopunch` from the desktop entry (Linux) / Start menu (Windows).
3. Connect (LocalAPI-only)
   - Verify it connects and shows which endpoint is selected (system/user/override).
   - If it shows `forbidden`, add your user to `miopunch-operators` (Linux) and re-login.
4. Core tasks
   - Run invite/join/ping tasks and observe task list updates from events.
5. Embedded terminal
   - Open the `sh_attach` terminal and verify interactive I/O + resize.
6. Reports
   - Open a completed task report and export it via the Save dialog.

## Packaging notes (v0)

- Windows
  - Install dir: `%ProgramFiles%\\miopunch\\`
  - Installer log: `%ProgramData%\\miopunch\\install.log` (exportable from installer UI)
  - Install: fail-fast on `miopunch install-system-daemon` errors
  - Uninstall: best-effort `miopunch uninstall-system-daemon`, then removes binaries/shortcuts; state preserved
- Linux
  - Binaries: `/usr/bin/miopunch`, `/usr/bin/miopunch-desktop`
  - Installer log: `/var/log/miopunch/install.log`
  - `apt remove`: preserves `/var/lib/miopunch`
  - `apt purge`: removes `/var/lib/miopunch` and `/var/log/miopunch`
