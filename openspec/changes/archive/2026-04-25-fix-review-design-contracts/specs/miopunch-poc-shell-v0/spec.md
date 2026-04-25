## ADDED Requirements

### Requirement: sh_attach interactive CLI exits when remote session ends
The interactive `sh_attach` CLI SHALL restore the local terminal and return when the task WebSocket closes, the remote task ends, or the WebSocket read path fails. It MUST NOT wait for an additional local stdin byte before exiting. After the WebSocket ends, the CLI SHALL still make a bounded best-effort attempt to fetch final task state and export the report when requested.

#### Scenario: Remote close while stdin is idle
- **WHEN** a user is attached with `miopunch sh <peer> ...`
- **AND** the task WebSocket closes while the user is not typing
- **THEN** the CLI restores the terminal and returns without waiting for stdin input
- **AND** final task-state lookup and report export remain bounded
