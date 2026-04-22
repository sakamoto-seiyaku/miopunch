# miopunch-code-health Specification

## Purpose
TBD - created by archiving change go-code-review-fixups. Update Purpose after archive.
## Requirements
### Requirement: QUIC control Close releases underlying connection
The system SHALL ensure that the `io.ReadWriteCloser` returned by the control plane (`internal/control`) for `quic` releases all underlying QUIC resources when `Close()` is invoked (including the QUIC connection, not only the stream), so repeated experiment runs do not leak goroutines or file descriptors.

#### Scenario: Closing a QUIC control session
- **WHEN** a peer establishes a control plane session using `control-proto=quic`
- **AND** the peer closes the returned session handle
- **THEN** the underlying QUIC connection is closed and no further stream/conn IO is possible

### Requirement: No stdout/stderr output from library packages
The system SHALL NOT write debug or operational logs directly to stdout/stderr from non-`cmd/` packages. Non-CLI packages MUST use the repo logging facility (e.g. `internal/logutil`) and/or structured event output (`event.Emitter`) so that machine-parsed event streams are not polluted by arbitrary text.

#### Scenario: Event output remains machine-parseable
- **WHEN** a user runs `miopunch peer ...` and captures stdout as an event stream
- **THEN** stdout contains only newline-delimited JSON `event.Event` records (no `fmt.Printf` debug lines)

### Requirement: QUIC ALPN naming converges to miopunch
The system SHALL use `miopunch` namespaced QUIC ALPN strings for both control plane and data plane, and MUST NOT introduce new `xtcp` naming into runtime protocol identifiers.

#### Scenario: QUIC handshake uses miopunch ALPN
- **WHEN** a user establishes control/data plane sessions via QUIC
- **THEN** the negotiated ALPN contains `miopunch` and does not contain `xtcp`

