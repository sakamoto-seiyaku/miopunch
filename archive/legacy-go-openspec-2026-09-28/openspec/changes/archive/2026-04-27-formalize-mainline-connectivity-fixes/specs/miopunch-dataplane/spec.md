## ADDED Requirements

### Requirement: Dataplane exposes peer transport sessions and logical streams
After traversal establishes a usable path, the dataplane SHALL establish a secure peer transport session before exposing application payload I/O.

The session SHALL support per-operation logical streams. Closing a logical stream SHALL NOT close the underlying peer transport session unless the session manager explicitly closes the session.

#### Scenario: Short operation closes only its logical stream
- **GIVEN** a peer transport session has been established
- **WHEN** a short operation such as ping completes
- **THEN** the operation's logical stream is closed
- **AND** the peer transport session remains eligible for later logical streams

### Requirement: Logical stream opening is generic
Each logical stream SHALL be opened with a generic kind and metadata envelope before kind-specific payload is exchanged.

The stream kind SHALL NOT be hard-coded to shell protocol operations; `shellproto` SHALL be one payload protocol carried by a logical stream kind.

#### Scenario: Shell payload uses a logical stream kind
- **WHEN** the caller opens a shell operation stream
- **THEN** the stream-open envelope identifies a shell kind
- **AND** shell-specific frames are exchanged only after the stream has been authorized
