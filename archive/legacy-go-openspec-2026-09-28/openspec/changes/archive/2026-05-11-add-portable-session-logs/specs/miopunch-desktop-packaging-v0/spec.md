## ADDED Requirements

### Requirement: Session bundles provide local runtime diagnostics
The current Windows and Linux portable/session bundles SHALL write runtime logs
into a `logs` directory beside the extracted bundle binaries.

The desktop GUI runtime log SHALL be written to `logs/miopunch-desktop.log`.

The session daemon runtime log SHALL be written to `logs/miopunch.log` when
`miopunch up --session` is launched directly or by `miopunch-desktop`.

The bundled smoke instructions SHALL identify these log paths and SHALL provide
an ordered manual test sequence covering launch, LocalAPI connection,
invite/join, peer visibility, ping, and shell attach.

Desktop task event handling SHALL preserve the final task facts needed by the
manual smoke sequence. In particular, a successful Create Invite task SHALL
surface a concrete `invite_code` fact in the desktop UI even when intermediate
task fact events are coalesced or missed by the runtime event stream.

On Linux, if the desktop runtime cannot initialize GTK or no display session is
available, `miopunch-desktop` SHALL report actionable guidance instead of
showing only a Go panic stack. The failure SHALL also be written to
`logs/miopunch-desktop.log` when the log path is writable.

#### Scenario: Desktop session writes local logs
- **WHEN** a user launches `miopunch-desktop` from an extracted session bundle
- **THEN** the session bundle contains `logs/miopunch-desktop.log`
- **AND** a desktop-managed daemon writes `logs/miopunch.log` in the same bundle

#### Scenario: Bundle smoke instructions include diagnostics and peer tests
- **WHEN** the session bundle is built
- **THEN** its `SMOKE.md` identifies the local log files
- **AND** it describes how to test two extracted bundles through invite/join,
  peer refresh, ping, and shell attach without requiring a system service

#### Scenario: Desktop invite code remains visible after coalesced task events
- **WHEN** a successful invite task emits its final task event to the desktop UI
- **THEN** the UI can render the real invite code from the task snapshot
- **AND** it does not leave the invite input empty while only showing
  `miopunch join <invite_code>` placeholder guidance

#### Scenario: Linux GTK initialization failure is diagnosable
- **WHEN** `miopunch-desktop` cannot initialize GTK on Linux
- **THEN** the process prints guidance about display session and GTK/WebKitGTK
  runtime checks
- **AND** the failure is recorded in `logs/miopunch-desktop.log` when possible
