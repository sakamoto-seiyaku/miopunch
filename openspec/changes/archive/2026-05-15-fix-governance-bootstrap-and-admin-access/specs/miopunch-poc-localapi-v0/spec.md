## ADDED Requirements

### Requirement: LocalAPI supports local network initialization task
`POST /api/v0/tasks` SHALL support task kind `init_network`.

The task args SHALL include `mode`, where supported values are `bootstrap` and
`create_new`. `create_new` SHALL require confirmation value
`create-new-network`.

#### Scenario: Desktop client initializes blank network through a task
- **WHEN** a desktop or CLI client creates `init_network` with `mode=bootstrap`
  on a blank node
- **THEN** LocalAPI creates a task
- **AND** the completed task reports the new local `net_id` and `peer_id`

#### Scenario: Missing confirmation rejects create-new
- **WHEN** a client creates `init_network` with `mode=create_new` without the
  required confirmation
- **THEN** the task fails with `BAD_REQUEST`
- **AND** existing local governance state is preserved

### Requirement: Desktop state exposes local governance capabilities
`GET /api/v0/desktop/state` and desktop runtime events SHALL expose non-secret
local governance capability state under the desktop config/state model.

The state SHALL include the governance classification, self role, whether the
node can initialize owner mode, whether it can create a new network, and whether
invite/approve actions are available.

#### Scenario: Blank node exposes initialization capability
- **WHEN** a desktop client loads state for a blank node
- **THEN** the desktop state reports governance state `no_network`
- **AND** `can_init_owner=true`
- **AND** admin invite/approve capabilities are false until initialization

#### Scenario: Admin node exposes invite approval capability
- **WHEN** a desktop client loads state for an admin network
- **THEN** the desktop state reports governance state `admin_network`
- **AND** invite and approve capabilities are true

#### Scenario: Non-admin node exposes create-new capability
- **WHEN** a desktop client loads state for member, foreign, or stale local
  governance state
- **THEN** invite and approve capabilities are false
- **AND** create-new-network capability is true
