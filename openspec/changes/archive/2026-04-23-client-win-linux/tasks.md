## 1. Scaffolding

- [x] 1.1 Create `cmd/miopunch-desktop` Wails app skeleton wired into repo build
- [x] 1.2 Define how `internal/http_panel/assets/` is reused by the desktop app (no CDN; minimal/no Node toolchain for v0)
- [x] 1.3 Add a minimal developer run path for desktop app (how to run against `miopunch up`)

## 2. LocalAPI Bridge (Go)

- [x] 2.1 Implement LocalAPI probe order system→user with visible selected endpoint
- [x] 2.2 Implement advanced LocalAPI override (equivalent to CLI `--localapi`, bypasses probe order, clearable)
- [x] 2.3 Implement connection failure classification (`bad_request/forbidden/daemon_not_running/unknown`) and map to actionable suggestions
- [x] 2.4 Expose snapshot methods to UI (status/peers/tasks/task/report) via Wails bindings
- [x] 2.5 Implement global events pump (consume `GET /api/v0/events` snapshot-first, emit runtime events to UI)
- [x] 2.6 Implement report export (UI chooses path; bridge fetches report and writes file)

## 3. Embedded Terminal Bridge

- [x] 3.1 Implement loopback-only WS server with random port + random token
- [x] 3.2 Proxy xterm WS frames to LocalAPI `sh_attach` WS (require `miopunch.sh.v0`)
- [x] 3.3 Support terminal resize control frames (winsize) end-to-end
- [x] 3.4 Add tests for WS token enforcement and basic proxy behavior

## 4. Frontend Adaptation (MD3 UI reuse)

- [x] 4.1 Replace `fetch/EventSource` usage with Wails bindings + runtime events (keep UI layout)
- [x] 4.2 Update `sh_attach` frontend to connect to the loopback-only WS bridge (token + reconnect basics)
- [x] 4.3 Update report export UI to use desktop bridge (save dialog + write file) instead of browser download
- [x] 4.4 Ensure the desktop UI is self-contained (no external network dependencies) and keeps MD3 baseline styling

## 5. Windows Packaging (NSIS)

- [x] 5.1 Ensure Windows build uses `wails build -webview2 embed` and document runtime expectations
- [x] 5.2 Implement NSIS installer: copy `miopunch.exe` + `miopunch-desktop.exe`, call `miopunch install-system-daemon`, fail-fast on error
- [x] 5.3 Implement installer log at `%ProgramData%\\miopunch\\install.log` and add “export log” UI
- [x] 5.4 Implement uninstall semantics: best-effort `miopunch uninstall-system-daemon`, continue removing binaries/shortcuts, preserve state
- [x] 5.5 Make Windows stable-binary install idempotent when already stable (avoid self-delete when running from `%ProgramFiles%\\miopunch\\miopunch.exe`)

## 6. Linux Packaging (.deb)

- [x] 6.1 Create `.deb` packaging that installs `/usr/bin/miopunch` and `/usr/bin/miopunch-desktop`
- [x] 6.2 Produce two `.deb` variants: WebKitGTK 4.0 (default) and WebKitGTK 4.1 (`-tags webkit2_41`), with appropriate runtime deps
- [x] 6.3 Add desktop entry + icons (display name `miopunch`, Exec `miopunch-desktop`)
- [x] 6.4 Implement `postinst`: call `miopunch install-system-daemon` (fail-fast) and always print manual `miopunch-operators` instructions (best-effort add group if possible)
- [x] 6.5 Implement `prerm`: call `miopunch uninstall-system-daemon` (continue only on “not installed”, otherwise fail-fast)
- [x] 6.6 Implement remove vs purge semantics for `/var/lib/miopunch` and `/var/log/miopunch`
- [x] 6.7 Append install/uninstall diagnostics to `/var/log/miopunch/install.log` and mention path on failures
- [x] 6.8 Align Linux system daemon stable path with `.deb` (`/usr/bin/miopunch`) and make stable-binary install idempotent when already stable

## 7. Verification & Docs

- [x] 7.1 Add unit tests for bridge error classification and LocalAPI events parsing
- [x] 7.2 Add a minimal manual smoke checklist (install → open GUI → connect → invite/join/ping/sh → export report)
- [x] 7.3 Add short packaging notes for Windows/Linux (paths, logs, repair/uninstall semantics)
