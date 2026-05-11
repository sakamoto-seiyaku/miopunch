## 1. Spec And Baseline Alignment

- [x] 1.1 Update desktop docs/README smoke wording so current Door 1 mainline is session-first and does not require system service install.
- [x] 1.2 Identify all current desktop smoke/build references that call `install-system-daemon` or require admin/root and mark which belong to deferred `D1a-privileged`.
- [x] 1.3 Add or update test fixtures for the new desktop connection states: session connected, bootstrapping, desktop-managed daemon, reused daemon, and bootstrap failure.

## 2. Desktop Session Bootstrap

- [x] 2.1 Extend `internal/desktopbridge` connection state to represent selected endpoint, bootstrap progress, desktop-managed daemon ownership, and bootstrap diagnostics.
- [x] 2.2 Change default desktop connection order to prefer same-user LocalAPI, then same-user daemon bootstrap, with override still bypassing default selection.
- [x] 2.3 Implement sibling `miopunch up` process launch from `miopunch-desktop` with bounded readiness wait and captured stderr/stdout diagnostics.
- [x] 2.4 Ensure system LocalAPI permission failures are retained as diagnostic facts but do not block same-user bootstrap.
- [x] 2.5 Stop only desktop-managed daemon processes on explicit application quit; preserve reused daemons.

## 3. Daemon Session Behavior

- [x] 3.1 Adjust Linux `miopunch up` probing so inaccessible system LocalAPI does not block non-root user LocalAPI startup.
- [x] 3.2 Adjust Windows `miopunch up` session behavior so desktop-launched non-admin daemon can serve the same-user LocalAPI endpoint.
- [x] 3.3 Add daemon tests for duplicate detection, inaccessible system endpoint fallback, and same-user endpoint readiness.
- [x] 3.4 Verify existing LocalAPI task/event/report/`sh_attach` behavior is unchanged for desktop-managed session daemons.

## 4. Window Residency

- [x] 4.1 Enable Wails single-instance behavior for `miopunch-desktop`.
- [x] 4.2 On second launch, restore and focus the existing window without leaving a second GUI process running.
- [x] 4.3 Implement Windows close-to-hide/resident behavior with an explicit quit path.
- [x] 4.4 Implement Linux tray-first close behavior and safe close-to-exit fallback when reliable tray support is unavailable.

## 5. Packaging Session Path

- [x] 5.1 Update Windows NSIS scaffold so service install/uninstall commands are disabled, guarded, or clearly commented as deferred privileged behavior for current session smoke.
- [x] 5.2 Update Linux `.deb` maintainer scripts so service install/uninstall commands are disabled, guarded, or clearly commented as deferred privileged behavior for current session smoke.
- [x] 5.3 Add session bundle build logic that emits `miopunch_<version>_windows_amd64_session.zip` containing `miopunch-desktop.exe`, `miopunch.exe`, license/notice files when present, and smoke instructions.
- [x] 5.4 Add session bundle build logic that emits `miopunch_<version>_linux_amd64_session.tar.gz` containing executable `miopunch-desktop`, executable `miopunch`, license/notice files when present, and smoke instructions.
- [x] 5.5 Update release/build artifact workflow so the Windows zip and Linux tar.gz session bundles are uploaded as copyable test artifacts.
- [x] 5.6 Add build-time archive checks that fail if a session bundle is missing the GUI binary, sibling daemon/CLI binary, or smoke instructions.
- [x] 5.7 Update packaging docs to distinguish current session bundles from future `D1a-privileged` installer/package delivery.

## 6. Verification

- [x] 6.1 Run focused Go tests for `internal/desktopbridge` and `cmd/miopunch` daemon startup behavior.
- [x] 6.2 Run desktop browser tests for connection state, diagnostics, override behavior, and runtime updates.
- [x] 6.3 Run static checks proving current Windows/Linux session bundle paths do not invoke `install-system-daemon` or `uninstall-system-daemon`.
- [x] 6.4 Build the Linux session tar.gz locally with `export PATH=/usr/local/go/bin:$PATH` and verify the archive contents and executable bits.
- [x] 6.5 Build the Windows session zip locally or in CI and verify the archive contents.
- [ ] 6.6 Run Windows true-machine smoke: extract the session zip as a normal user, launch `miopunch-desktop.exe`, verify it starts/reuses sibling `miopunch.exe`, connects to LocalAPI, and supports the core desktop task flow.
- [ ] 6.7 Run Linux true-machine smoke: extract the session tar.gz as a normal user, run `./miopunch-desktop`, verify it starts/reuses sibling `./miopunch`, connects to LocalAPI, and supports the core desktop task flow.
- [x] 6.8 Before mainline merge, run the full `$dev` validation gate set for code-affecting changes.
