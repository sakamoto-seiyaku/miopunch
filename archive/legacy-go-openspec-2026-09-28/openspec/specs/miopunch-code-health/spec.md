# miopunch-code-health Specification

## Purpose
Defines repository-level code health constraints that remain active for the current POC v1 mainline.
## Requirements
### Requirement: No stdout/stderr output from library packages
The system SHALL NOT write debug or operational logs directly to stdout/stderr from non-`cmd/` packages. Non-CLI packages MUST use the repo logging facility (e.g. `internal/logutil`) and/or structured event output (`event.Emitter`) so that machine-parsed event streams are not polluted by arbitrary text.

#### Scenario: Event output remains machine-parseable
- **WHEN** a user runs current POC v1 CLI commands with machine-readable output
- **THEN** stdout contains only the requested CLI response or structured event records
- **AND** library debug output does not pollute stdout or stderr

### Requirement: Dispatcher handler access is race-free and terminal errors are observable
The system SHALL ensure internal asynchronous message dispatchers provide concurrency-safe handler registration and lookup. Dispatcher handler registration MUST be safe before and after `Run()`. Dispatcher `Send` MUST mean the message was accepted for asynchronous sending, not that the underlying wire write completed. When dispatcher read or write loops terminate because of an error, that terminal error MUST be observable by callers after the dispatcher is done.

#### Scenario: Handler registration after Run is safe
- **WHEN** a dispatcher has started its read and write loops
- **AND** a caller registers a message handler while the dispatcher may be reading messages
- **THEN** the registration and message dispatch do not race or panic
- **AND** matching messages are delivered to the registered handler

#### Scenario: Write failure closes dispatcher with observable error
- **WHEN** the dispatcher send loop fails to write a message to the underlying connection
- **THEN** the dispatcher closes its done signal
- **AND** callers can observe the terminal write error

### Requirement: Event emission write failures are checkable
The system SHALL ensure structured event emission returns JSON encode or writer errors to callers. Callers MAY ignore the returned error for best-effort diagnostics, but the emitter itself MUST NOT discard the error internally without making it available to the caller.

#### Scenario: Failing event writer returns an error
- **WHEN** an event emitter writes to a writer that returns an error
- **THEN** the event emission call returns an error
- **AND** the caller can distinguish that event output failed

### Requirement: Reviewed runtime resources are bounded and cleaned up
The system SHALL ensure current POC v1 runtime, UDP traversal, KCP/TLS/yamux session, LocalAPI, and shell paths do not leave goroutines, wait paths, or owned network resources stuck after early failure.

#### Scenario: Failed POC v1 session establishment cleans up owned resources
- **WHEN** UDP path establishment or secure-session upgrade fails before a peer session is live
- **THEN** owned temporary sockets, transports, streams, and wait paths are closed or canceled
- **AND** runtime-owned UDP sockets remain open until the runtime owner closes them

#### Scenario: Shell attach failure does not strand process or stream state
- **WHEN** a current POC v1 shell attach or remote shell command fails during setup
- **THEN** the shell/session setup returns an observable error
- **AND** any subprocess, stream, or session resource owned by that attempt is cleaned up

### Requirement: Reviewed boundary failures fail closed
The system SHALL fail reviewed security and boundary-validation errors closed instead of panicking or falling back to predictable values. Attempt setup MUST return an error for a nil exchange response. Session ID generation MUST NOT produce timestamp-only IDs when cryptographic random generation fails.

#### Scenario: Nil NatHole response returns an error
- **WHEN** attempt setup receives a nil `NatHoleResp`
- **THEN** it returns a normal error
- **AND** it does not panic

#### Scenario: SID generation does not fall back to timestamp-only IDs
- **WHEN** cryptographic random ID generation fails during NAT-hole session setup
- **THEN** the session setup fails with an observable error
- **AND** the system does not create a timestamp-only session ID

### Requirement: Go source remains gofmt-clean
The system SHALL keep checked-in Go source formatted with `gofmt`.

#### Scenario: gofmt reports no repository drift
- **WHEN** `gofmt -l` runs over checked-in Go source
- **THEN** it reports no files requiring formatting
