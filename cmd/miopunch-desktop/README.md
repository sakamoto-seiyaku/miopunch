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
2. Launch the GUI from the extracted directory.
   - Windows: open `miopunch-desktop.exe`.
   - Linux: run `./miopunch-desktop`.
3. Connect (LocalAPI-only)
   - Verify the GUI reuses an existing same-user LocalAPI or starts the sibling `miopunch up`.
   - Verify the UI shows the selected endpoint and whether the daemon is desktop-managed.
4. Core tasks
   - Run invite/join/ping tasks and observe task list updates from events.
5. Embedded terminal
   - Open the `sh_attach` terminal and verify interactive I/O + resize.
6. Reports
   - Open a completed task report and export it via the Save dialog.

## Packaging notes (session v0)

- Current smoke artifacts are portable session bundles, not privileged installers.
- Each session bundle contains `miopunch-desktop`, sibling `miopunch`, license/notice
  files when present, and smoke instructions.
- NSIS and `.deb` scaffolds remain in the repo for the deferred `D1a-privileged`
  route. They are not the current session smoke gate and must not be required to
  run `install-system-daemon` or `uninstall-system-daemon`.
