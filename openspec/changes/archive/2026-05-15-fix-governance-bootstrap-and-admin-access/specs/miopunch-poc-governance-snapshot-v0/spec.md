## ADDED Requirements

### Requirement: Local governance lifecycle is explicit
The system SHALL classify local governance state before exposing owner/admin
capabilities.

The classification SHALL distinguish at least:

- `no_network`: no local network or governance trust root exists.
- `admin_network`: local network and governance head exist and the current
  identity is an owner or admin.
- `member_network`: local network and governance head exist, the current
  identity is not an admin, and local evidence indicates it is a member.
- `foreign_or_stale_network`: local network/governance state exists but the
  current identity is neither admin nor a proven member, or local state is
  inconsistent.

#### Scenario: Blank local state is classified as no network
- **WHEN** the node has no local net and no governance head
- **THEN** local governance state is `no_network`
- **AND** the node may initialize a new owner/admin network

#### Scenario: Existing non-admin state is not promotable
- **WHEN** a local governance head exists and the current identity is not an
  owner or admin
- **THEN** local governance state is not `admin_network`
- **AND** the node SHALL NOT treat the current identity as an admin of that
  existing network

### Requirement: Network initialization creates a real local trust root
The system SHALL provide an explicit local initialization action for a blank
node.

The action SHALL create or reuse the local identity, create a local net, create
a genesis governance head with the current identity as owner/admin, and ensure
an empty declaration set.

The action SHALL fail without side effects if local network or governance state
already exists.

#### Scenario: Blank node initializes owner admin network
- **GIVEN** a node is classified as `no_network`
- **WHEN** the user initializes the current node as owner/admin
- **THEN** the node persists a new net
- **AND** the governance head lists the current identity as owner and admin
- **AND** later state is classified as `admin_network`

#### Scenario: Existing network blocks bootstrap initialization
- **GIVEN** a node already has local network or governance state
- **WHEN** the user requests blank-node initialization
- **THEN** the action fails
- **AND** the existing network and governance files are not replaced

### Requirement: Creating a new network does not promote stale identities
The system SHALL provide an explicit confirmed action to create a new local
network when existing local state is stale, foreign, or no longer admin-capable.

Creating a new network SHALL generate a new net ID and genesis governance head
for the current identity. It SHALL NOT add the current identity as admin to the
previous local governance head.

#### Scenario: Stale state creates a distinct new network
- **GIVEN** a node has local network or governance state where the current
  identity is not admin
- **WHEN** the user confirms creating a new network
- **THEN** the node persists a different net ID
- **AND** the new governance head lists the current identity as owner/admin
- **AND** old member declarations and bootstrap recommendations are not carried
  into the new network

#### Scenario: New network creation requires confirmation
- **WHEN** the user requests creating a new network without the required
  confirmation
- **THEN** the action fails with a user-fixable error
- **AND** existing local network and governance files are not replaced
