## ADDED Requirements

### Requirement: Desktop shell supports a repeatable demo loop
The desktop GUI SHALL provide a repeatable shell demo loop using the existing `sh_ls` and `sh_attach` task contracts.

The loop SHALL allow a user to select an operable peer, list available shell targets or sessions, choose target and session values, connect an embedded terminal, disconnect, and reconnect without leaving the peer shell view.

When discovery has not returned a richer value, the GUI SHALL default to target `local` and session `main`.

#### Scenario: User lists sessions before connecting
- **GIVEN** a remote peer is operable
- **WHEN** the user opens the peer shell view and starts discovery
- **THEN** the GUI creates an `sh_ls` task for that peer
- **AND** the shell view shows available target or session choices when the task result provides them

#### Scenario: User connects, disconnects, and reconnects
- **GIVEN** a remote peer is operable
- **WHEN** the user connects a shell with selected target and session values
- **THEN** the GUI creates an `sh_attach` task with those values
- **AND** the embedded terminal shows connected status
- **WHEN** the user disconnects
- **THEN** the terminal transport is closed and the shell view can reconnect with the same selected values

### Requirement: Desktop shell failures are visible and recoverable
The desktop GUI SHALL show recoverable failure states for shell discovery, `sh_attach` task creation, terminal bridge setup, WebSocket close, and terminal library load failures.

After a recoverable failure, the shell view SHALL keep the selected peer, target, and session values available for retry.

#### Scenario: Discovery failure keeps retry available
- **WHEN** the user starts shell discovery and the `sh_ls` task or bridge call fails
- **THEN** the GUI shows a visible failure message
- **AND** discovery can be retried for the same peer

#### Scenario: Attach failure keeps retry available
- **WHEN** the user starts shell attach and task creation or terminal bridge setup fails
- **THEN** the GUI shows a visible failure message
- **AND** Connect becomes available again for the same peer, target, and session

### Requirement: Desktop shell demo loop has browser coverage
The desktop GUI SHALL include browser-level tests for the committed static UI shell demo loop.

The tests SHALL use fake bridge/runtime behavior and SHALL NOT require live network transport.

#### Scenario: Browser test covers successful shell loop
- **WHEN** the browser test opens an operable peer shell view
- **THEN** it can run discovery, connect `sh_attach`, observe terminal status, disconnect, and reconnect
- **AND** the fake bridge records the expected `sh_ls` and `sh_attach` task calls

#### Scenario: Browser test covers shell failure recovery
- **WHEN** the fake bridge fails discovery or attach setup
- **THEN** the browser test observes a visible failure state
- **AND** the shell control can be retried without navigating away
