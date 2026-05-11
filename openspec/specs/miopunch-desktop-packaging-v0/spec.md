# miopunch-desktop-packaging-v0 Specification

## Purpose
TBD - created by archiving change client-win-linux. Update Purpose after archive.
## Requirements
### Requirement: Desktop delivery includes both daemon and GUI binaries
The current session-first desktop delivery shape SHALL include:
- `miopunch` (CLI + daemon)
- `miopunch-desktop` (GUI)

The current supported smoke shape SHALL be a portable/session bundle where `miopunch-desktop` can locate and start the sibling `miopunch` binary without requiring administrator/root installation.

The current session artifacts SHALL be named:
- `miopunch_<version>_windows_amd64_session.zip`
- `miopunch_<version>_linux_amd64_session.tar.gz`

The delivery SHALL NOT distribute `miopunch-desktop` alone as a supported method.

#### Scenario: Portable session bundle delivers two executables
- **WHEN** a user obtains the current desktop session bundle
- **THEN** both `miopunch` and `miopunch-desktop` are present in the bundle
- **AND** the user can launch `miopunch-desktop` without first installing a system service

#### Scenario: Windows session bundle can be launched without install
- **WHEN** a Windows user extracts `miopunch_<version>_windows_amd64_session.zip`
- **THEN** the extracted directory contains `miopunch-desktop.exe` and `miopunch.exe`
- **AND** the user can start testing by launching `miopunch-desktop.exe`
- **AND** no installer or Administrator prompt is required

#### Scenario: Linux session bundle can be launched without install
- **WHEN** a Linux user extracts `miopunch_<version>_linux_amd64_session.tar.gz`
- **THEN** the extracted directory contains executable `miopunch-desktop` and `miopunch`
- **AND** the user can start testing by running `./miopunch-desktop`
- **AND** no package installation or root prompt is required

### Requirement: Desktop GUI resolves daemon from the extracted session bundle
For current session bundles, `miopunch-desktop` SHALL locate the daemon/CLI binary in the same extracted directory before falling back to any system path.

If the sibling daemon/CLI binary is missing or not executable, the GUI SHALL show a bundle integrity diagnostic and SHALL NOT suggest `install-system-daemon` as the default fix.

#### Scenario: GUI starts sibling daemon from Windows bundle
- **WHEN** a Windows user launches `miopunch-desktop.exe` from the extracted session bundle
- **THEN** the GUI starts or reuses the sibling `miopunch.exe` for the same user session
- **AND** it does not require `%ProgramFiles%\\miopunch\\miopunch.exe`

#### Scenario: GUI starts sibling daemon from Linux bundle
- **WHEN** a Linux user launches `./miopunch-desktop` from the extracted session bundle
- **THEN** the GUI starts or reuses the sibling `./miopunch` for the same user session
- **AND** it does not require `/usr/bin/miopunch`

#### Scenario: Missing sibling daemon gives bundle diagnostic
- **WHEN** the user launches `miopunch-desktop` from a session bundle missing the sibling daemon/CLI binary
- **THEN** the GUI reports that the bundle is incomplete
- **AND** the default suggested fix is to re-extract or rebuild the session bundle

### Requirement: Session bundle build automation publishes copyable artifacts
The build automation SHALL create the current Windows and Linux session artifacts and expose them as build outputs suitable for copying to real test machines.

The artifacts SHALL include the GUI binary, daemon/CLI binary, license/notice files when present, and a short smoke README explaining how to launch without installing.

#### Scenario: Build outputs Windows session artifact
- **WHEN** the desktop session artifact build runs for Windows amd64
- **THEN** it writes `miopunch_<version>_windows_amd64_session.zip`
- **AND** the archive contains `miopunch-desktop.exe`, `miopunch.exe`, and smoke instructions

#### Scenario: Build outputs Linux session artifact
- **WHEN** the desktop session artifact build runs for Linux amd64
- **THEN** it writes `miopunch_<version>_linux_amd64_session.tar.gz`
- **AND** the archive contains executable `miopunch-desktop`, executable `miopunch`, and smoke instructions

### Requirement: Current packaging smoke disables privileged service orchestration
Current Windows/Linux desktop smoke scripts and documentation SHALL NOT require `miopunch install-system-daemon` or `miopunch uninstall-system-daemon`.

If existing NSIS or `.deb` scaffolding remains in the repository, service-install and service-uninstall sections SHALL be disabled, guarded behind an explicit non-default privileged route, or clearly commented as deferred `D1a-privileged` behavior.

#### Scenario: Windows session smoke does not invoke service install
- **WHEN** the current Windows desktop smoke packaging path is used
- **THEN** it does not invoke `miopunch install-system-daemon`
- **AND** it does not require Administrator privileges

#### Scenario: Linux session smoke does not invoke service install
- **WHEN** the current Linux desktop smoke packaging path is used
- **THEN** it does not invoke `miopunch install-system-daemon`
- **AND** it does not require root privileges

### Requirement: Windows WebView2 runtime strategy uses embedded bootstrapper in v0
The Windows desktop GUI SHALL be built with `-webview2 embed`.

If WebView2 Runtime is missing and the runtime cannot be installed/bootstrapped, the GUI SHALL show actionable guidance and SHALL exit.

#### Scenario: Missing WebView2 runtime results in actionable guidance
- **GIVEN** WebView2 Runtime is not installed on the machine
- **AND** the machine cannot install the runtime
- **WHEN** the user launches `miopunch-desktop`
- **THEN** the app shows actionable guidance to install WebView2 Runtime
- **AND** the app exits

### Requirement: Portable session bundles keep runtime data beside the bundle
The current Windows and Linux portable/session bundles SHALL store session runtime data under a `data` directory beside the extracted bundle binaries.

The default portable session state path SHALL be `data/state.json` under the extracted bundle directory. State-derived files such as `net.json`, `identity/`, `decls/`, `bootstrap/`, and task `reports/` SHALL also reside under `data/`.

When `miopunch-desktop` starts the sibling daemon from a session bundle, it SHALL start it with the bundle-local session state path.

When a user manually runs `miopunch up --session` from the extracted bundle and does not provide `--state_path`, the daemon SHALL use the same bundle-local session state path.

An explicit `--state_path` override SHALL continue to take precedence over the portable session default.

#### Scenario: Desktop-managed daemon writes data into the extracted bundle
- **WHEN** a user launches `miopunch-desktop` from an extracted session bundle
- **AND** the GUI starts the sibling daemon
- **THEN** the daemon uses `<bundle>/data/state.json` as its state path
- **AND** identity, network, peer, bootstrap, and report files are written under `<bundle>/data/`

#### Scenario: Manual session daemon writes data into the extracted bundle
- **WHEN** a user runs `miopunch up --session` from an extracted session bundle
- **AND** no `--state_path` override is provided
- **THEN** the daemon uses `<bundle>/data/state.json` as its state path

#### Scenario: Explicit state path override is preserved
- **WHEN** a user runs `miopunch up --session --state_path <custom-state>`
- **THEN** the daemon uses `<custom-state>`
- **AND** it does not replace the custom path with `<bundle>/data/state.json`

#### Scenario: Session bundle smoke docs identify local data
- **WHEN** the session bundle is built
- **THEN** its smoke instructions identify `data/state.json`
- **AND** they state that removing `data/` resets the portable node for a clean smoke run

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
