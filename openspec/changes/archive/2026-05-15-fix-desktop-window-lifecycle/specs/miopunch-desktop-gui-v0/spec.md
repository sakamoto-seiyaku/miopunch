## MODIFIED Requirements

### Requirement: Desktop GUI is single-instance with platform window residency semantics
The desktop GUI SHALL enforce a single running GUI instance per user session.

On a second launch, the existing GUI window SHALL be restored and focused instead of starting an independent GUI process.

On Windows, closing the main window SHALL ask the user whether to keep the GUI running in the system tray or quit the application. Choosing the tray option SHALL hide the window and keep the GUI resident with a visible tray affordance. Choosing quit SHALL fully close the GUI.

Linux close semantics SHALL prefer tray-backed hide/resident behavior when a reliable tray is available; if no reliable tray is available, closing the window SHALL exit the application.

#### Scenario: Second launch restores existing window
- **GIVEN** `miopunch-desktop` is already running
- **WHEN** the user launches `miopunch-desktop` again
- **THEN** the existing window is shown and focused
- **AND** no second independent GUI instance remains running

#### Scenario: Windows close can keep the app in tray
- **WHEN** a Windows user closes the desktop window
- **AND** chooses to keep running in the tray
- **THEN** the window is hidden
- **AND** the desktop session remains resident
- **AND** the user can restore the window from the tray affordance

#### Scenario: Windows close can fully quit
- **WHEN** a Windows user closes the desktop window
- **AND** chooses to quit
- **THEN** the GUI exits instead of remaining hidden
- **AND** normal desktop shutdown handling runs

#### Scenario: Windows tray quit fully exits
- **GIVEN** the Windows desktop GUI is resident in the tray
- **WHEN** the user activates Quit from the tray menu
- **THEN** the tray affordance is removed
- **AND** the GUI exits
- **AND** normal desktop shutdown handling runs

#### Scenario: Windows tray context menu exposes restore and quit
- **GIVEN** the Windows desktop GUI is resident in the tray
- **WHEN** the user right-clicks the tray affordance
- **THEN** a tray menu is shown
- **AND** the menu includes Open and Quit actions

#### Scenario: Windows tray activation restores the GUI
- **GIVEN** the Windows desktop GUI is resident in the tray
- **WHEN** the user activates the tray affordance by click, double-click, or keyboard selection
- **THEN** the existing GUI window is shown, restored from minimised or hidden state, and brought forward

#### Scenario: Linux without reliable tray exits safely
- **GIVEN** the Linux desktop environment has no reliable tray support
- **WHEN** the user closes the desktop window
- **THEN** the application exits instead of remaining hidden without a recovery affordance

### Requirement: Desktop GUI owns only desktop-managed daemon shutdown
When `miopunch-desktop` starts a same-user session daemon, it SHALL track that daemon as desktop-managed.

The GUI SHALL stop a desktop-managed daemon on explicit application quit or Windows full close. The GUI SHALL NOT stop a daemon that it merely reused.

#### Scenario: Explicit quit stops desktop-managed daemon
- **GIVEN** the GUI started a same-user session daemon
- **WHEN** the user explicitly quits the desktop application
- **THEN** the GUI best-effort stops the desktop-managed daemon

#### Scenario: Windows full close stops desktop-managed daemon
- **GIVEN** the GUI started a same-user session daemon
- **WHEN** a Windows user closes the window and chooses to quit
- **THEN** the GUI best-effort stops the desktop-managed daemon

#### Scenario: Tray quit stops desktop-managed daemon
- **GIVEN** the GUI started a same-user session daemon
- **AND** the GUI is resident in the Windows tray
- **WHEN** the user activates Quit from the tray menu
- **THEN** the GUI best-effort stops the desktop-managed daemon

#### Scenario: Explicit quit preserves reused daemon
- **GIVEN** the GUI connected to a daemon that was already running
- **WHEN** the user explicitly quits the desktop application
- **THEN** the GUI closes without stopping that daemon
