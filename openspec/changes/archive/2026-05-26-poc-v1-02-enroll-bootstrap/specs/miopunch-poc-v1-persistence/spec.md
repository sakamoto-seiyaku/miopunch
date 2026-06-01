## MODIFIED Requirements

### Requirement: Current v1 file responsibilities are fixed
The system SHALL assign these responsibilities to the current v1 layout:

- `device/ed25519.key`
- `device/x25519.key`
- `device/enroll_handled/<network_id>/<msg_id>.json`
- `networks/<network_id>/member_credential.bin`
- `networks/<network_id>/mailbox_secret.bin`
- `networks/<network_id>/roster_snapshot.json`
- `networks/<network_id>/broker.json`

#### Scenario: Bootstrap and runtime state land in predictable files
- **WHEN** current v1 enroll, discover, or GUI state is persisted
- **THEN** each object is written to its fixed file role
- **AND** callers do not invent per-feature ad hoc files

## ADDED Requirements

### Requirement: Authority replay cache is device-scoped and not joined visibility
The system SHALL persist authority-side enroll replay records under `device/enroll_handled/<network_id>/<msg_id>.json`.

This replay cache SHALL be durable across authority restart, but SHALL NOT make a joined network visible by itself and SHALL NOT be treated as part of the four-file joined bootstrap success unit.

#### Scenario: Replay cache exists before any joined-network bootstrap state
- **WHEN** authority persists a handled enroll request for one `network_id`
- **THEN** later replay lookup succeeds across restart
- **AND** joined-network reads still behave as absent until the four bootstrap files exist
