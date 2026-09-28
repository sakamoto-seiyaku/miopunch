## ADDED Requirements

### Requirement: Current v1 runtime owns UDP owner lifecycle
The current v1 Runtime SHALL own the lifecycle of its long-lived UDP owner and the underlying Runtime UDP socket.

Runtime SHALL create, retain, and close the long-lived UDP owner as a Runtime resource.

Punch and secure-session layers MAY borrow owner-provided traversal or packet transport views, but SHALL NOT close the Runtime UDP owner.

#### Scenario: Runtime closes UDP owner only at runtime shutdown
- **WHEN** a current v1 peer session closes after a successful Runtime-owned UDP handoff
- **THEN** Runtime's UDP owner remains open
- **AND** the owner is closed only when Runtime itself shuts down or explicitly replaces the UDP owner

#### Scenario: Failed handoff leaves Runtime UDP owner usable
- **WHEN** secure-session establishment fails after a Runtime-owned UDP punch succeeds
- **THEN** Runtime keeps the UDP owner usable for the next dial or accept attempt
- **AND** subsequent local candidates do not advertise a closed UDP file descriptor

### Requirement: Current v1 runtime exposes punch-to-secure-session failure evidence
The current v1 Runtime SHALL expose and log actionable evidence when a selected UDP path fails during secure-session handoff.

The evidence SHALL include:

- `remote_peer_id`
- `selected_path`
- selected remote UDP endpoint
- whether the selected UDP path was Runtime-owned or temporary
- secure-session error stage

#### Scenario: Accept-side secure-session failure is visible
- **WHEN** inbound punch handling selects a UDP path
- **AND** secure-session accept fails
- **THEN** Runtime logs or exposes failure evidence with the selected path and remote UDP endpoint
- **AND** the failure is distinguishable from punch failure

#### Scenario: Dial-side secure-session failure remains stage-locatable
- **WHEN** outbound punch succeeds but secure-session dial fails
- **THEN** CLI/runtime failure output identifies `SecureSession` as the failing stage
- **AND** facts include selected UDP path evidence from the preceding punch

### Requirement: Current v1 runtime reports UDP6 selected path evidence
When current v1 Runtime establishes a UDP6 direct path, it SHALL expose selected path evidence consistently with UDP4 paths.

#### Scenario: UDP6 direct path is operator-visible
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP6 direct reachability
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=direct_ipv6`
