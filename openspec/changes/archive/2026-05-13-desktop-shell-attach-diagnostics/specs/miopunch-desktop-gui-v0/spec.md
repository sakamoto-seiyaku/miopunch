## ADDED Requirements

### Requirement: Desktop shell abnormal close shows structured diagnostics
The desktop GUI SHALL surface the best available structured diagnostic summary
when an embedded desktop shell closes unexpectedly after attach setup has
started, instead of showing only a generic disconnect string.

The summary SHALL prefer final shell task diagnostics when available and SHALL
preserve the selected peer, target, and session so the operator can retry the
same shell path.

The summary SHALL distinguish user-requested disconnect from abnormal shell
termination.

#### Scenario: Unexpected shell close uses final task diagnostics
- **GIVEN** the desktop GUI has attached an embedded shell for a selected peer,
  target, and session
- **WHEN** the shell closes unexpectedly and the final `sh_attach` task exposes
  structured diagnostic output
- **THEN** the desktop shell view shows a concise failure summary derived from
  that diagnostic output instead of only a raw WebSocket close string
- **AND** Connect remains available for the same peer, target, and session

#### Scenario: Explicit disconnect is not shown as a failure
- **GIVEN** the desktop GUI has an active embedded shell
- **WHEN** the user explicitly disconnects that shell
- **THEN** the shell view returns to a disconnected state without an abnormal
  failure banner
- **AND** the same peer, target, and session remain selected for reconnect
