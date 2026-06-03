## ADDED Requirements

### Requirement: Android control-lite packages the current miopunch CLI
The system SHALL provide a demo Android APK that packages the current `cmd/miopunch` Android arm64 executable and runs it from the installed app without downloading code at runtime.

#### Scenario: APK contains an executable miopunch payload
- **WHEN** the Android control-lite debug APK is built for `arm64-v8a`
- **THEN** the APK contains a packaged `miopunch` native payload derived from the current `cmd/miopunch` tree
- **AND** the installed app can execute that payload to display `miopunch --help`

### Requirement: Android control-lite owns an app-lifetime runtime process
The system SHALL let the user start and stop a local `miopunch up` process from the APK using app-private state, LocalAPI, and log paths.

#### Scenario: User starts the runtime
- **WHEN** the user taps `Start Runtime`
- **THEN** the app launches `miopunch up` with `--localapi unix:<cacheDir>/miopunch-localapi.sock` and `--state_path <filesDir>/state/state.json`
- **AND** the app shows runtime stdout/stderr in its log surface

#### Scenario: User stops the runtime
- **WHEN** the user taps `Stop`
- **THEN** the app terminates the runtime process and any active shell process it owns
- **AND** the app updates the visible runtime state to stopped

### Requirement: Android control-lite executes control actions through the packaged CLI
The system SHALL expose minimal controls for `join`, `ls`, `ping`, and `sh ls` by launching the packaged `miopunch` binary against the app-owned LocalAPI endpoint.

#### Scenario: User joins a network from the phone
- **WHEN** the runtime is running and the user submits an invite code
- **THEN** the app runs `miopunch --localapi <app-localapi> join <invite_code>`
- **AND** the app displays the CLI result or failure facts without rewriting the backend reason code

#### Scenario: User validates the remote shell target
- **WHEN** the user has entered a peer ID and target
- **THEN** the app can run `ls`, `ping <peer_id>`, and `sh ls <peer_id> <target>`
- **AND** successful results preserve CLI evidence such as `reason_code=OK` and selected path facts when present

### Requirement: Android control-lite opens a simple interactive remote shell
The system SHALL let the user open a remote shell by launching `miopunch sh <peer_id> <target> -s <session>` and connecting the app shell input/output to that subprocess.

#### Scenario: User runs safe shell commands from the phone
- **WHEN** the user opens shell for a validated Linux/WSL peer and submits line-oriented commands such as `date`, `whoami`, `pwd`, or `ls`
- **THEN** the app writes the commands to the shell subprocess stdin
- **AND** the app displays remote stdout in an in-app terminal surface that can render ANSI shell output
- **AND** the app displays shell subprocess stderr or non-zero exit failures in the log surface

### Requirement: Android control-lite remains control-only
The system SHALL NOT expose the Android device as a remote shell target or advertise Android shell control as part of this demo.

#### Scenario: Peer lists shell targets
- **WHEN** a remote peer lists shell targets for the Android control-lite device
- **THEN** this APK demo does not add an Android-controlled target beyond whatever the underlying CLI runtime already exposes
- **AND** the demo documentation describes Android as the operator/control side only

### Requirement: Android control-lite has a repeatable demo runbook
The system SHALL include a short runbook for the known Linux/WSL and Pixel 6a demo path.

#### Scenario: Demo evidence is captured
- **WHEN** the APK demo is validated against the Linux/WSL peer
- **THEN** the runbook identifies the build, install, broker, invite, approve, ping, shell, and evidence capture steps
- **AND** the captured evidence includes Android logs, Linux daemon logs, and the successful remote shell commands
