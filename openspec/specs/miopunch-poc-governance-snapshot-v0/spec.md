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
