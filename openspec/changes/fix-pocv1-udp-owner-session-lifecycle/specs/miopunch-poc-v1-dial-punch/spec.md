## MODIFIED Requirements

### Requirement: PathResult is a borrowed UDP path descriptor
The system SHALL return `PathResult` as the selected UDP path descriptor after
current POC v1 dial/punch.

`PathResult` SHALL carry the remote UDP endpoint, trusted remote identity, and
selected-path evidence. If it carries a local UDP connection reference for the
current secure-session recipe, that reference SHALL be borrowed from Runtime and
SHALL NOT be closed by `PathResult.Close`.

#### Scenario: PathResult cleanup does not close the daemon UDP port
- **GIVEN** POC v1 dial/punch selected a path using Runtime's UDP socket
- **WHEN** the caller invokes `PathResult.Close` after a failed session upgrade
- **THEN** the daemon UDP socket remains open
- **AND** Runtime can use the same local UDP port for a retry
