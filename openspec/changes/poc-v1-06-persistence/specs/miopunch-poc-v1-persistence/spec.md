# miopunch-poc-v1-persistence Specification

## Purpose
定义当前 POC v1 的本地状态布局、原子写、权限规则与 typed persist authority。

## ADDED Requirements

### Requirement: Current v1 persistence root is caller-supplied
The system SHALL open current v1 persistence from an explicit root directory supplied by the caller.

This capability SHALL NOT infer its root from legacy `state.json` paths or other POC v0 global discovery rules.

#### Scenario: Persistence opens against an explicit root
- **WHEN** a current v1 runtime initializes persistence
- **THEN** it receives an explicit root directory from its caller
- **AND** it does not derive that root by reusing legacy v0 state-path rules

### Requirement: Current v1 state layout is device plus per-network roots
The system SHALL store current v1 device-global state under `device/` and network-scoped state under `networks/<network_id>/`.

For current v1, `<network_id>` MUST be the canonical external `network_id`: the unpadded uppercase base32 encoding of the raw 16-byte network value, yielding exactly 26 ASCII characters.

#### Scenario: Device and network state are separated
- **WHEN** a current v1 runtime persists long-lived state
- **THEN** device keys live under `device/`
- **AND** each network's state lives under `networks/<network_id>/`

### Requirement: Current v1 file responsibilities are fixed
The system SHALL assign these responsibilities to the current v1 layout:

- `device/ed25519.key`
- `device/x25519.key`
- `networks/<network_id>/member_credential.bin`
- `networks/<network_id>/mailbox_secret.bin`
- `networks/<network_id>/roster_snapshot.json`
- `networks/<network_id>/broker.json`

#### Scenario: Bootstrap and runtime state land in predictable files
- **WHEN** current v1 enroll, discover, or GUI state is persisted
- **THEN** each object is written to its fixed file role
- **AND** callers do not invent per-feature ad hoc files

### Requirement: Current v1 joined bootstrap persistence is grouped and atomic
The system SHALL persist bootstrap success for one joined network through one persistence-owned grouped write operation containing:

- `member_credential.bin`
- `mailbox_secret.bin`
- `broker.json`
- `roster_snapshot.json`

The grouped write SHALL make the joined network either fully visible or absent after failure or restart.

#### Scenario: Failed bootstrap persistence does not expose a partially joined network
- **WHEN** the runtime fails during the grouped write for one joined network
- **THEN** later readers do not observe a network directory that contains only a strict subset of the bootstrap files
- **AND** restart either sees the complete joined state or no joined state at all

### Requirement: Current v1 trusted roster is stored as a whole snapshot
The system SHALL persist `roster_snapshot.json` as one whole trusted roster payload.

This capability SHALL provide whole-read and whole-replace behavior for `roster_snapshot` and SHALL NOT define per-entry patch or merge semantics inside persistence.

#### Scenario: Trusted roster replacement is whole-snapshot only
- **WHEN** a current v1 component updates the trusted roster
- **THEN** it replaces the whole persisted roster snapshot for that network
- **AND** persistence does not reinterpret the update as an incremental per-peer merge

### Requirement: Current v1 runtime broker config is singular
The system SHALL persist exactly one current v1 runtime broker endpoint per joined network in `broker.json`.

The current extracted v1 SHALL NOT require a primary/secondary broker pair inside this capability.

#### Scenario: Joined state stores one runtime broker endpoint
- **WHEN** a current v1 joiner persists bootstrap success
- **THEN** `broker.json` stores exactly one runtime broker endpoint for that network
- **AND** later callers do not infer an additional backup broker from this persisted shape

### Requirement: Current v1 topic scope is derived by persistence
The system SHALL derive current v1 topic scope through a persistence-owned `TopicScope` API.

`TopicScope` SHALL accept canonical `network_id`, decode it back to raw 16 bytes as `network_id_bytes`, and derive topics from that raw value.
It SHALL NOT hash or HKDF against the UTF-8 bytes of the textual `network_id`.

For current v1 the derivation SHALL use:

- `network_id_bytes = base32_decode_upper_no_pad(network_id)`
- `salt = sha256(network_id_bytes)[:16]`
- `net_root = lower(base32(raw,no-pad, HKDF-SHA256(ikm=mailbox_secret, salt=salt, info="miopunch/v1/net_root", L=16)))`
- `inbox = lower(base32(raw,no-pad, HKDF-SHA256(ikm=mailbox_secret, salt=salt, info="miopunch/v1/topic.inbox/"+peer_id, L=16)))`
- `presence_topic = "mp/v1/net/<net_root>/presence/<peer_id>"`
- `inbox_topic = "mp/v1/net/<net_root>/inbox/<inbox>"`

#### Scenario: Topic scope is deterministic for the same network and peer
- **WHEN** current v1 persistence derives topic scope twice with the same canonical `network_id`, `mailbox_secret`, and canonical `peer_id`
- **THEN** it produces the same `net_root`, `presence_topic`, and `inbox_topic`
- **AND** callers do not reimplement a competing topic derivation rule outside persistence

### Requirement: Current v1 state writes are atomic
The system SHALL write current v1 state files atomically using temporary files plus rename.

#### Scenario: Partial rewrite is never observed
- **WHEN** a current v1 state file is rewritten
- **THEN** the runtime promotes a complete temporary file atomically
- **AND** readers never observe a partial final file

### Requirement: Current v1 state permissions are restrictive
The system SHALL create current v1 state directories with `0700` and state files with `0600`.

#### Scenario: Persisted secrets stay permission-locked
- **WHEN** the runtime creates or repairs a current v1 state directory or file
- **THEN** directories use `0700`
- **AND** files use `0600`
