## ADDED Requirements

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
The system SHALL ensure reviewed connectivity and dataplane paths do not leave goroutines, wait paths, or owned network connections stuck after early failure. TCP punching workers MUST have a deterministic stop path when target construction fails. TLS stream setup MUST close owned TCP candidates if TLS configuration fails before handshakes begin.

#### Scenario: Invalid TCP punching targets do not strand workers
- **WHEN** TCP punching cannot build attempt targets from the coordinator response
- **THEN** the attempt returns the target-build error
- **AND** no TCP punching worker remains blocked waiting for jobs

#### Scenario: TLS config failure closes candidate connections
- **WHEN** TCP dataplane convergence receives candidate TCP connections
- **AND** pinned TLS configuration fails before handshakes start
- **THEN** all owned candidate TCP connections are closed before returning the error

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
