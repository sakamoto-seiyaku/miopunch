## Context

The current session bundle contains only binaries and short launch notes. The
desktop app can start the sibling daemon, but the daemon still initializes
logging to console, and the desktop entrypoint does not initialize a persistent
runtime log. This leaves true-machine smoke dependent on terminal capture or UI
diagnostics when Windows/Linux behavior differs.

## Goals / Non-Goals

**Goals:**

- Put runtime logs next to the extracted session bundle so testers can archive
  or share them directly.
- Use stable filenames for both GUI and daemon logs.
- Add enough smoke instructions to move from "app launches" to "two machines
  can invite/join and exercise ping/shell".
- Preserve the current session-first bundle and desktop-managed daemon model.

**Non-Goals:**

- Do not add installer/system-service logging behavior.
- Do not change control-plane, dataplane, invite/join, ping, or shell semantics.
- Do not add a GUI log viewer in this change.

## Decisions

- Logs are written to `<bundle-dir>/logs/miopunch-desktop.log` and
  `<bundle-dir>/logs/miopunch.log`.
  - Rationale: "beside the extracted bundle" is deterministic even if a
    platform launcher changes the process working directory.
  - Alternative considered: write to the current working directory. That is
    easier to describe but can drift when launched through shortcuts or desktop
    shells.
- Runtime code computes the bundle directory from `os.Executable()` and creates
  `logs/` on startup.
  - Rationale: both desktop and desktop-managed daemon run from sibling
    binaries in the same directory, and this does not require extra flags.
- The bundled `SMOKE.md` becomes the operator-facing checklist for true-machine
  tests.
  - Rationale: the instructions travel with the artifact copied to Windows and
    Linux machines.

## Risks / Trade-offs

- [Risk] The extracted directory may be read-only. -> Mitigation: keep startup
  failure output on stderr/message box paths where applicable; tests should
  extract to a user-writable directory.
- [Risk] File logs can grow during long manual testing. -> Mitigation: reuse the
  existing rotating file writer with daily rotation.
- [Risk] Logs near binaries may not be suitable for future installed packages.
  -> Mitigation: scope this to session/portable bundles; privileged packaging
  can keep separate system log paths later.
