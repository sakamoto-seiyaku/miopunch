## MODIFIED Requirements

### Requirement: Runtime-owned UDP sockets are not closed by borrowed views
The system SHALL treat the POC v1 daemon UDP socket as owned by the runtime or
its socket-owner abstraction.

Borrowed views, demux endpoints, path results, and secure-session transports
SHALL NOT close the runtime UDP socket. They MAY close their own in-memory
session/listener/endpoint state.

#### Scenario: Session close leaves runtime UDP available
- **GIVEN** a runtime-owned UDP socket has been handed to punch/session code
- **WHEN** a path result, secure-session attempt, or peer session is closed
- **THEN** the runtime UDP socket remains usable
- **AND** later punch/session attempts do not fail because the same UDP pointer references a closed file descriptor
