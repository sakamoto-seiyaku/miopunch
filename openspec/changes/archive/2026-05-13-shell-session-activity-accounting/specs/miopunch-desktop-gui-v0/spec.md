## ADDED Requirements

### Requirement: Embedded shell terminal focuses input when opened
The desktop GUI SHALL focus the embedded shell terminal input when the terminal
is opened for a shell session.

The user SHALL be able to start typing into a newly opened shell session
without an extra click inside the terminal area.

#### Scenario: Connected shell is ready for immediate input
- **WHEN** the user opens an embedded shell session in the desktop GUI
- **AND** the shell reaches its connected terminal view
- **THEN** the terminal input is focused
- **AND** keyboard input can be sent immediately without a separate focus click
