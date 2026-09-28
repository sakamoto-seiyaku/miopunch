## ADDED Requirements

### Requirement: Desktop delivery includes both daemon and GUI binaries
The system SHALL provide a desktop delivery shape that includes:
- `miopunch` (CLI + daemon)
- `miopunch-desktop` (GUI)

The delivery SHALL NOT distribute `miopunch-desktop` alone as a supported installation method.

#### Scenario: Installer/package delivers two executables
- **WHEN** a user installs miopunch via the supported desktop installer/package
- **THEN** both `miopunch` and `miopunch-desktop` are installed on the machine

### Requirement: Windows installer uses NSIS and delegates service install to miopunch
On Windows, the system SHALL provide an NSIS installer that:
- Installs to `%ProgramFiles%\\miopunch\\`
- Copies `miopunch.exe` and `miopunch-desktop.exe` into that directory
- Calls `miopunch install-system-daemon` to register and start the system service

If `miopunch install-system-daemon` fails, the installer SHALL fail-fast and SHALL present actionable diagnostics.

#### Scenario: Successful Windows install registers and starts the daemon service
- **WHEN** a user runs the Windows installer with Administrator privileges
- **THEN** the installer copies `miopunch.exe` and `miopunch-desktop.exe` into `%ProgramFiles%\\miopunch\\`
- **AND** the installer calls `miopunch install-system-daemon`
- **AND** the system daemon service is registered and started

#### Scenario: Windows install fails when daemon service install fails
- **WHEN** the installer calls `miopunch install-system-daemon`
- **AND** the command fails
- **THEN** the installer fails the installation
- **AND** the UI shows actionable diagnostics and where to find the installer log

### Requirement: Windows WebView2 runtime strategy uses embedded bootstrapper in v0
The Windows desktop GUI SHALL be built with `-webview2 embed`.

If WebView2 Runtime is missing and the runtime cannot be installed/bootstrapped, the GUI SHALL show actionable guidance and SHALL exit.

#### Scenario: Missing WebView2 runtime results in actionable guidance
- **GIVEN** WebView2 Runtime is not installed on the machine
- **AND** the machine cannot install the runtime
- **WHEN** the user launches `miopunch-desktop`
- **THEN** the app shows actionable guidance to install WebView2 Runtime
- **AND** the app exits

### Requirement: Windows uninstall is best-effort for daemon service and preserves state
The Windows uninstaller SHALL best-effort call `miopunch uninstall-system-daemon` (Administrator privileges required).

If `miopunch uninstall-system-daemon` fails, the uninstaller SHALL still continue removing GUI binaries and shortcuts, and SHALL present a warning with actionable guidance.

The uninstall flow SHALL preserve miopunch state (aligned with `uninstall-system-daemon` semantics).

#### Scenario: Uninstall continues even if daemon uninstall fails
- **WHEN** the Windows uninstaller attempts to call `miopunch uninstall-system-daemon`
- **AND** the command fails
- **THEN** the uninstaller continues removing `%ProgramFiles%\\miopunch\\miopunch.exe` and `%ProgramFiles%\\miopunch\\miopunch-desktop.exe`
- **AND** the uninstaller shows a warning with next-step guidance

### Requirement: Windows installer writes install logs and supports exporting them
The Windows installer SHALL append install/repair logs to `%ProgramData%\\miopunch\\install.log`.

The installer UI SHALL provide an option to export the installer log to a user-selected path.

#### Scenario: User exports installer log to a chosen path
- **GIVEN** an installer log exists at `%ProgramData%\\miopunch\\install.log`
- **WHEN** the user clicks “Export log” and selects an output path
- **THEN** the installer writes a copy of the log to that path

### Requirement: Linux .deb package delegates service install to miopunch and is fail-fast on install
On Linux, the system SHALL provide a `.deb` package as the initial desktop packaging format.

The package SHALL:
- Install `miopunch` to `/usr/bin/miopunch`
- Install `miopunch-desktop` to `/usr/bin/miopunch-desktop`
- Provide a desktop entry named `miopunch` whose Exec points to `miopunch-desktop`
- In `postinst`, call `miopunch install-system-daemon`

If `miopunch install-system-daemon` fails in `postinst`, the package installation SHALL fail (fail-fast).

#### Scenario: Debian package install registers and starts daemon service
- **WHEN** a user installs the `.deb` package with root privileges
- **THEN** `/usr/bin/miopunch` and `/usr/bin/miopunch-desktop` are installed
- **AND** `postinst` calls `miopunch install-system-daemon`
- **AND** the system daemon service is registered and started

#### Scenario: Debian package install fails when install-system-daemon fails
- **WHEN** `postinst` calls `miopunch install-system-daemon`
- **AND** the command fails
- **THEN** the package installation fails
- **AND** the output indicates where to find install logs and how to fix permissions

### Requirement: Linux .deb provides WebKitGTK 4.0 and 4.1 variants
The Linux desktop delivery SHALL provide two `.deb` variants:
- a WebKitGTK 4.0 variant (default)
- a WebKitGTK 4.1 variant (for newer distros that require 4.1; built with `-tags webkit2_41`)

The variants SHALL NOT both be installed at the same time.

Both variants SHALL declare runtime dependencies that include:
- `libgtk-3-0` (or equivalent)
- an appropriate `libwebkit2gtk` runtime package for the selected variant

#### Scenario: WebKitGTK 4.0 variant depends on libwebkit2gtk-4.0
- **WHEN** inspecting the WebKitGTK 4.0 variant `.deb` metadata
- **THEN** the dependencies include `libwebkit2gtk-4.0` (or equivalent)

#### Scenario: WebKitGTK 4.1 variant depends on libwebkit2gtk-4.1
- **WHEN** inspecting the WebKitGTK 4.1 variant `.deb` metadata
- **THEN** the dependencies include `libwebkit2gtk-4.1` (or equivalent)

### Requirement: Linux operator group guidance is always printed and is best-effort applied
The Linux install flow SHALL treat `miopunch-operators` as the operator group name.

The `.deb` install flow SHALL best-effort add the inferred operator user to the `miopunch-operators` group (when the operator user can be inferred), but SHALL NOT block installation solely due to inability to infer a non-root operator user.

The install flow SHALL always print actionable instructions for manually adding a user to `miopunch-operators` and re-login.

#### Scenario: Installer prints manual group-join instructions
- **WHEN** the `.deb` package installation completes (successfully)
- **THEN** the installer output includes instructions to add a user to `miopunch-operators` and re-login

### Requirement: Linux uninstall semantics distinguish remove vs purge
On Linux, the packaging SHALL distinguish:
- `apt remove`: removes binaries and desktop entry, but preserves system state (e.g. `/var/lib/miopunch`)
- `apt purge`: additionally removes system state (e.g. `/var/lib/miopunch`) and logs (e.g. `/var/log/miopunch`)

#### Scenario: apt remove preserves system state
- **GIVEN** miopunch has created system state under `/var/lib/miopunch`
- **WHEN** the user runs `apt remove` for miopunch
- **THEN** the binaries and desktop entry are removed
- **AND** `/var/lib/miopunch` remains on disk

#### Scenario: apt purge removes system state and logs
- **GIVEN** miopunch has created system state under `/var/lib/miopunch` and logs under `/var/log/miopunch`
- **WHEN** the user runs `apt purge` for miopunch
- **THEN** `/var/lib/miopunch` is removed
- **AND** `/var/log/miopunch` is removed

### Requirement: Linux uninstall invokes uninstall-system-daemon with fail-fast semantics except not-installed
The Linux packaging uninstall flow (`prerm`) SHALL call `miopunch uninstall-system-daemon`.

If `miopunch uninstall-system-daemon` reports “not installed/not found”, the uninstall flow SHALL continue.
For other failures, the uninstall flow SHALL fail-fast.

#### Scenario: Uninstall continues when daemon service is not installed
- **WHEN** `prerm` calls `miopunch uninstall-system-daemon`
- **AND** the command reports that the service is not installed
- **THEN** the package uninstall continues

### Requirement: Linux install/uninstall writes an installer log
The Linux install/uninstall flow SHALL append logs to `/var/log/miopunch/install.log`.

On install/uninstall failures, the output SHALL mention the installer log path.

#### Scenario: Failure output points to /var/log/miopunch/install.log
- **WHEN** a package install or uninstall step fails
- **THEN** the output includes `/var/log/miopunch/install.log` as the location for diagnostics
