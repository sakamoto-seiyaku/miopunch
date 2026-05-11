## MODIFIED Requirements

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

## ADDED Requirements

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

## REMOVED Requirements

### Requirement: Windows installer uses NSIS and delegates service install to miopunch
**Reason**: The current desktop mainline is session-first and must not require Administrator privileges or system service registration for smoke validation.
**Migration**: Move this requirement to a later `D1a-privileged` change when installer-first service management returns to scope.

### Requirement: Windows uninstall is best-effort for daemon service and preserves state
**Reason**: Current session-first smoke does not install a Windows system service.
**Migration**: Restore or redefine this requirement in `D1a-privileged` together with the privileged Windows installer route.

### Requirement: Windows installer writes install logs and supports exporting them
**Reason**: Current session-first smoke is not gated on the NSIS installer.
**Migration**: Keep runtime diagnostics in the session route; restore installer-log requirements when privileged installer delivery is reintroduced.

### Requirement: Linux .deb package delegates service install to miopunch and is fail-fast on install
**Reason**: The current Linux desktop mainline must be testable without root privileges or system service registration.
**Migration**: Move `.deb` service-install requirements to `D1a-privileged`.

### Requirement: Linux .deb provides WebKitGTK 4.0 and 4.1 variants
**Reason**: `.deb` delivery is no longer the current acceptance gate for Session v0.
**Migration**: Reintroduce `.deb` variant requirements in `D1a-privileged` or a later release-packaging change.

### Requirement: Linux operator group guidance is always printed and is best-effort applied
**Reason**: Session v0 does not depend on the `miopunch-operators` system group.
**Migration**: Restore operator-group guidance when system daemon packaging returns.

### Requirement: Linux uninstall semantics distinguish remove vs purge
**Reason**: Current session-first smoke does not install or uninstall a system package.
**Migration**: Restore package remove/purge semantics in the future privileged packaging change.

### Requirement: Linux uninstall invokes uninstall-system-daemon with fail-fast semantics except not-installed
**Reason**: Current session-first smoke must not call system daemon uninstall.
**Migration**: Restore this requirement with `.deb` service management in `D1a-privileged`.

### Requirement: Linux install/uninstall writes an installer log
**Reason**: Current session-first smoke is not gated on `.deb` maintainer scripts.
**Migration**: Restore installer-log requirements when `.deb` becomes a current delivery route again.
