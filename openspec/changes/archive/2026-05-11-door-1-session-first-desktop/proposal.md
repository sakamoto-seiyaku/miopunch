## Why

Door 1 Pro desktop mainline has moved from installer-first to session-first, but the current specs and packaging scripts still make admin/root installation and system service registration part of the default desktop validation path. This blocks fast Windows/Linux real-device smoke and makes automation depend on privileged install flows that are only needed for later virtual networking work.

## What Changes

- Make `miopunch-desktop` the single default user entry for the current desktop mainline.
- Change the desktop default from "connect to an already-installed system daemon" to "reuse or bootstrap a same-user session daemon".
- Keep `LocalAPI-only` as the GUI/daemon boundary; no public network listener is introduced.
- Change connection diagnostics so current-session failures suggest retry/bootstrap/session troubleshooting, not installer repair or `install-system-daemon` as the default path.
- Add single-instance and window residency requirements for the current desktop session shape.
- Produce current session test artifacts that can be copied to real machines without installation:
  - `miopunch_<version>_windows_amd64_session.zip`
  - `miopunch_<version>_linux_amd64_session.tar.gz`
- Require each session artifact to contain both the GUI binary and sibling daemon/CLI binary so users can launch the GUI directly from the extracted directory.
- Downgrade NSIS/.deb service-install semantics from current acceptance criteria to the deferred privileged route.
- Require current packaging/smoke scripts to disable or clearly comment out `install-system-daemon` / `uninstall-system-daemon` orchestration in the session-first validation path.
- Defer admin/root installation, system service, stable system binary path, operator group governance, and virtual networking/TUN prerequisites to later `D1a-privileged`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: desktop GUI startup, LocalAPI selection, daemon bootstrap, diagnostics, single-instance, and window residency requirements change to session-first.
- `miopunch-desktop-packaging-v0`: current desktop delivery and smoke requirements change from installer/service-first to portable/session-first; privileged installer/package behavior is deferred.
- `miopunch-poc-daemon-up`: daemon foreground/session behavior must support non-admin desktop bootstrap without being blocked by inaccessible system LocalAPI endpoints.

## Impact

- Desktop code: `cmd/miopunch-desktop` and `internal/desktopbridge` connection/bootstrap lifecycle.
- Daemon code: `cmd/miopunch up` startup/probe behavior on Linux and Windows.
- Frontend/tests: desktop connection state, diagnostics, bridge fixtures, and browser smoke coverage.
- Packaging: Windows NSIS and Linux `.deb` scripts/docs must stop making service install a prerequisite for current session smoke.
- Release/build automation: session bundle scripts and CI artifacts must publish the Windows zip and Linux tar.gz directory packages as the current desktop smoke deliverables.
- Validation: this is code-affecting when applied; before mainline merge it requires focused desktop/daemon tests plus the full `$dev` gate set.
