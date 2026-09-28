## Context

`docs/decisions/door-1-pro-session-shell-charter.md` makes the current Door 1 Pro desktop mainline session-first: users open `miopunch-desktop`, and the GUI reuses or starts a same-user daemon without requiring admin/root install. The archived Door 1 desktop implementation and current specs still encode the older installer-first path: GUI connects to an existing daemon, packaging calls `install-system-daemon`, and smoke assumes system service registration.

The current code already has the main pieces needed for a smaller session-first change: `miopunch up`, IPC-only LocalAPI, desktop bridge code, Wails runtime bindings/events, and packaging scripts. The change is cross-cutting because the product contract, desktop bridge, daemon startup behavior, tests, and packaging scripts must agree on the same default path.

## Goals / Non-Goals

**Goals:**

- Make `miopunch-desktop` the only default user-facing entry for current desktop smoke.
- Let the GUI reuse a reachable LocalAPI or bootstrap a same-user session daemon when none is usable.
- Produce copyable Windows/Linux session artifacts that can be extracted and launched without installing miopunch.
- Keep GUI/daemon communication `LocalAPI-only`.
- Remove admin/root, system service, operator group, and installer success from current acceptance.
- Preserve the privileged installer/service route as a later `D1a-privileged` path.
- Make Windows/Linux true-machine smoke automatable without privilege prompts.

**Non-Goals:**

- Do not implement virtual networking, TUN, or service-managed privileged networking.
- Do not delete `install-system-daemon` / `uninstall-system-daemon`.
- Do not redesign LocalAPI routes, task semantics, report format, or `sh_attach`.
- Do not introduce Electron or a public TCP LocalAPI listener.
- Do not make `.deb`/NSIS the current release gate.

## Decisions

### 1) GUI owns the current session daemon lifecycle

`miopunch-desktop` will first try the configured override when present. Without override, it will prefer the same-user LocalAPI endpoint, then start a same-user `miopunch up` child process if no usable endpoint is reachable. If a daemon is already reachable, GUI reuses it and does not spawn another one.

For session bundles, the GUI must first resolve the sibling daemon binary from the same extracted directory:
- Windows: `miopunch.exe` next to `miopunch-desktop.exe`
- Linux: `miopunch` next to `miopunch-desktop`

If the sibling daemon is missing or not executable, the GUI should fail with a bundle integrity diagnostic instead of suggesting `install-system-daemon`.

Alternative considered: keep requiring users or installers to start the daemon first. This preserves the old implementation but keeps the two-step workflow and admin install blocker that the charter explicitly removes from the current mainline.

### 2) System LocalAPI is diagnostic, not a blocking default

The desktop bridge may still probe system LocalAPI to report that a privileged daemon exists, but system permission errors must not stop user-session bootstrap. This is required because a normal user may not be in `miopunch-operators`, and that should not block the current session-first product path.

Alternative considered: keep system-first selection. That makes installed service scenarios convenient but keeps the old privileged route as the default mental model.

### 3) Session daemon uses existing `miopunch up` with minimal process management

The GUI should start the sibling `miopunch` executable with `up` and let it bind the default same-user LocalAPI. The bridge waits for LocalAPI readiness with a bounded timeout, captures startup failure diagnostics, and records whether the daemon is desktop-managed. Explicit app quit should terminate a managed child process best-effort; reusing an existing daemon must not stop that daemon.

Alternative considered: move daemon runtime into `miopunch-desktop` as an in-process server. That would duplicate the daemon entrypoint and increase lifecycle/test risk.

### 4) Current packaging is portable/session-first

The current smoke artifacts are:
- `miopunch_<version>_windows_amd64_session.zip`
- `miopunch_<version>_linux_amd64_session.tar.gz`

Each artifact extracts to a single directory named after the artifact stem. The directory contains the GUI binary, the sibling daemon/CLI binary, license/notice files when present, and a short smoke README. Users start testing by opening `miopunch-desktop.exe` on Windows or running `./miopunch-desktop` on Linux.

Windows NSIS and Linux `.deb` scripts may remain in the repo, but their service-install sections must be commented out, disabled by a non-default guard, or otherwise excluded from the current session smoke path. Documentation must say these scripts are privileged-route scaffolding until `D1a-privileged`.

Alternative considered: immediately rewrite NSIS/.deb into non-admin installers. That is larger than needed for the interview/real-device validation goal and risks spending time on installer behavior instead of desktop session behavior.

### 5) Window residency is part of session behavior

Wails single-instance support should be used so a second `miopunch-desktop` launch restores/focuses the existing window. Windows close hides the window and keeps the session daemon alive. Linux uses tray-first behavior when reliable tray support is available; if no tray can be provided, close exits instead of leaving an unrecoverable hidden app.

Alternative considered: keep default close-to-exit everywhere. That is simpler but does not match the current Windows product shape and gives poor daemon lifecycle semantics.

## Risks / Trade-offs

- [Risk] Wails tray behavior varies by Linux desktop environment. -> Mitigation: require safe no-tray fallback that exits on close.
- [Risk] Child daemon process cleanup differs across OSes. -> Mitigation: only stop desktop-managed children; leave reused daemons alone; test graceful and forced cleanup separately.
- [Risk] Existing packaging scripts still encode privileged assumptions. -> Mitigation: add explicit script guards/comments and update docs/tests so current smoke cannot accidentally invoke service install.
- [Risk] A session bundle missing the sibling daemon would look like an app startup failure. -> Mitigation: make sibling binary presence/executability a build-time artifact check and a runtime diagnostic.
- [Risk] Windows named pipe user/system distinction is currently collapsed around operator SID. -> Mitigation: preserve the same IPC path for same-user session mode and avoid adding new pipe naming policy unless implementation proves it necessary.
- [Risk] Full gate set is expensive. -> Mitigation: run focused desktop/daemon checks during iteration, then run the required `$dev` full gate set before mainline merge.

## Migration Plan

1. Update specs so session-first is the current contract and privileged install/service requirements are deferred.
2. Implement desktop bridge bootstrap and lifecycle state.
3. Adjust `miopunch up` so same-user foreground startup is not blocked by inaccessible system endpoints.
4. Add session bundle build automation that emits the Windows zip and Linux tar.gz artifacts with the fixed names and layout above.
5. Disable/comment privileged service orchestration in current packaging smoke scripts and docs.
6. Add focused unit/browser/static packaging tests.
7. Run focused validation, then the full `$dev` gate set before merging code-affecting work.

Rollback is straightforward: keep `install-system-daemon` and the privileged packaging scaffolds intact, and revert the desktop default back to connect-only/system-service smoke if session bootstrap is not viable.
