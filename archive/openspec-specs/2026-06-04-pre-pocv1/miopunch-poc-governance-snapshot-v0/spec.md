# miopunch-poc-governance-snapshot-v0 Specification

## Purpose
`miopunch-poc-governance-snapshot-v0` defines the minimal owner-signed governance snapshot head used in POC v0.

POC-06.5 implements **genesis + acceptance/update verification semantics**; full propose/sign/apply CLI is deferred.
## Requirements
### Requirement: snapshot_body binds the network and height
For POC v0, `snapshot_body` SHALL include (at minimum):

- `net_id` (string)
- `height` (int; genesis=0; each update +1)
- `prev_hash_b64` (string; genesis="" / empty string)
- `owners` (list of Ed25519 public keys)
- `admins` (list of Ed25519 public keys)

#### Scenario: Snapshot body contains trust-root fields
- **WHEN** a node creates or validates a governance snapshot body
- **THEN** the body includes `net_id`, `height`, `prev_hash_b64`, `owners`, and `admins`
- **AND** the `net_id` binds the snapshot to the intended network

### Requirement: snapshot_body hash is canonical and stable
For POC v0, the governance snapshot head SHALL be represented by `snapshot_body` plus `signatures`.

The snapshot body hash SHALL be computed as:

- `hash_b64 = base64url(no-pad, sha256(canonical_json(snapshot_body)))`

`hash_b64` is for display and state alignment; receivers MUST recompute and verify it.

#### Scenario: Receiver recomputes the snapshot hash
- **WHEN** a receiver loads a governance snapshot head
- **THEN** it recomputes `hash_b64` from canonical JSON of `snapshot_body`
- **AND** it rejects or reports the snapshot if the supplied hash does not match

### Requirement: signatures cover snapshot_body hash
For POC v0, each signature entry SHALL be:

- `signature = { key_id, sig_b64 }`

Where:

- `key_id = hex(sha256(ed25519_pub))` (TUF-style key identifier; 64 hex chars)
- `sig_b64 = base64url(no-pad, Ed25519.Sign(owner_priv, sha256(canonical_json(snapshot_body))))`

#### Scenario: Owner signature verifies over snapshot body
- **WHEN** a receiver validates a signature entry in a governance snapshot
- **THEN** it derives `key_id` from the owner public key
- **AND** it verifies `sig_b64` over the canonical snapshot body hash

### Requirement: head snapshot is persisted on disk
Nodes SHALL persist the current governance head snapshot at:

- `governance/head_snapshot.json` under the node state directory.

#### Scenario: Node stores the current governance head
- **WHEN** a node accepts a governance head snapshot
- **THEN** it persists the snapshot at `governance/head_snapshot.json`
- **AND** the persisted snapshot can be loaded after restart

### Requirement: receiver validates updates with old+new thresholds
When applying a candidate snapshot and a local head snapshot exists, the node SHALL:

- reject updates whose `prev_hash_b64` does not match the local head hash (to avoid forks in POC v0)
- require `height == local_height+1`
- require `net_id == local_net_id`
- require **old-threshold (=1)**: at least one signature verifiable under the local head owners (old trust root)
- require **new-threshold (=1)**: at least one signature verifiable under the candidate owners (self-signed new trust root)

If the candidate hash equals the local head hash, the node MAY treat it as a no-op success.

#### Scenario: Candidate update must satisfy old and new trust roots
- **GIVEN** a node already has a local governance head snapshot
- **WHEN** it receives a candidate snapshot with `height == local_height+1`
- **THEN** it accepts the update only if `prev_hash_b64`, `net_id`, old-threshold, and new-threshold checks all pass

### Requirement: bootstrap acceptance uses self-signed snapshot
When accepting an initial snapshot and no local head snapshot exists (e.g., join bootstrap), the node SHALL validate the snapshot as self-contained:

- recompute and verify `hash_b64`
- require **new-threshold (=1)**: at least one signature verifiable under the candidate owners
- require `net_id == local_net_id` (derived from `net_secret`)

#### Scenario: Bootstrap accepts a self-contained initial snapshot
- **GIVEN** a node has no local governance head snapshot
- **WHEN** it receives an initial snapshot whose hash, new-threshold signature, and `net_id` are valid
- **THEN** it accepts the snapshot as the bootstrap governance head

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
