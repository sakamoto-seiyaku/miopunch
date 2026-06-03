## MODIFIED Requirements

### Requirement: Desktop GUI is single-instance with platform window residency semantics
The desktop GUI SHALL enforce a single running GUI instance per user session.

On a second launch, the existing GUI window SHALL be restored and focused
instead of starting an independent GUI process.

On Windows, closing the main window SHALL ask the user whether to keep the GUI
running in the system tray or quit the application. Choosing the tray option
SHALL hide the window and keep the GUI resident with a visible tray affordance.
Choosing quit SHALL fully close the GUI.

Linux close semantics SHALL prefer tray-backed hide/resident behavior when a
reliable tray is available; if no reliable tray is available, closing the window
SHALL exit the application.

Full desktop shutdown SHALL NOT remain blocked waiting for a LocalAPI runtime
event stream that was active when close was requested.

#### Scenario: Windows close can fully quit while events are streaming
- **GIVEN** the Windows desktop GUI has an active LocalAPI runtime event stream
- **WHEN** the user closes the window and chooses to quit
- **THEN** the GUI exits instead of remaining hidden or resident
- **AND** normal desktop shutdown handling runs

#### Scenario: Linux close exits while events are streaming
- **GIVEN** the Linux desktop GUI has an active LocalAPI runtime event stream
- **AND** no reliable tray affordance is available
- **WHEN** the user closes the desktop window
- **THEN** the application exits instead of waiting for terminal Ctrl+C

### Requirement: Desktop GUI owns only desktop-managed daemon shutdown
When `miopunch-desktop` starts a same-user session daemon, it SHALL track that
daemon as desktop-managed.

The GUI SHALL stop a desktop-managed daemon on explicit application quit,
Linux full close, or Windows full close. The GUI SHALL NOT stop a daemon that
it merely reused.

#### Scenario: Full close stops desktop-managed daemon after event stream cleanup
- **GIVEN** the GUI started a same-user session daemon
- **AND** the desktop runtime event stream is active
- **WHEN** the user fully closes the desktop application
- **THEN** the GUI best-effort stops the desktop-managed daemon
- **AND** the close path does not wait indefinitely for the event stream reader

#### Scenario: Full close preserves reused daemon
- **GIVEN** the GUI connected to a daemon that was already running
- **WHEN** the user fully closes the desktop application
- **THEN** the GUI closes without stopping that daemon
