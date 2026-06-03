# miopunch-poc-decls-v0 Specification

## Purpose
`miopunch-poc-decls-v0` defines the POC v0 declaration set (`decls`) used for membership convergence:

- `approve_member` adds/confirms a member identity
- `revoke_member` permanently revokes a member identity (tombstone)

The declaration set converges by **set-union**. Revoke is irreversible in POC v0.

## Requirements

### Requirement: decl wire shape is stable for POC v0
For POC v0, each decl SHALL have the following JSON fields:

- `msg_id` (string, 26 chars, base32(raw,no-pad))
- `created_at_unix_ms` (int64)
- `issuer_peer_id` (string, 26 chars)
- `kind` (string): `approve_member` or `revoke_member`
- `body` (object)
- `sig_b64` (string): base64url(no-pad) Ed25519 signature

#### Scenario: Node creates a stable decl object
- **WHEN** a node creates an `approve_member` or `revoke_member` decl
- **THEN** the decl contains `msg_id`, `created_at_unix_ms`, `issuer_peer_id`, `kind`, `body`, and `sig_b64`
- **AND** `kind` is either `approve_member` or `revoke_member`

### Requirement: decl signature is verifiable and deterministic
For POC v0, `sig_b64` MUST be computed over a deterministic transcript derived from the decl contents (excluding `sig_b64`).

Nodes MUST reject decls whose signature cannot be verified against the issuer key.

#### Scenario: Invalid decl signature is rejected
- **WHEN** a node receives a decl whose `sig_b64` does not verify against the issuer key
- **THEN** the node rejects the decl
- **AND** the decl is not added to the local decl set

### Requirement: decls converge by set-union
Nodes SHALL store `decls` as a set keyed by `msg_id` (duplicates are ignored).

#### Scenario: Duplicate decl is ignored
- **WHEN** a node receives the same decl more than once
- **THEN** the node stores only one entry for that `msg_id`
- **AND** the decl set remains convergent by set-union

### Requirement: revoke_member is a permanent tombstone
If a `revoke_member` decl exists for a given member identity, the node SHALL treat that identity as revoked permanently (cannot be re-approved with the same identity key).

#### Scenario: Revoke overrides later approve
- **WHEN** a decl set contains a `revoke_member` decl for a member identity
- **AND** an `approve_member` decl for the same identity is present or later received
- **THEN** the node treats that identity as revoked

### Requirement: decls head hash can be reported
Nodes SHALL be able to report a `decls_head_b64` summary hash computed from the decl set contents to detect divergence and trigger best-effort synchronization.

#### Scenario: Different decl sets produce different heads
- **WHEN** two nodes report `decls_head_b64` for different decl set contents
- **THEN** the reported hashes differ
- **AND** the system can use that difference to trigger best-effort synchronization

### Requirement: approve_member carries reachability hints for peer selection
An `approve_member` declaration SHALL be able to carry `v4_hint` and `v6_hint` values for the approved member.

Reachability hints SHALL be used only for ordering bootstrap and neighbor candidates. Hints MUST NOT contain endpoint addresses, ports, or private network details.

The v4 hint order SHALL be:
`direct > easy > hard1 > hard2 > unknown > none`.

The v6 hint order SHALL be:
`direct > easy > hard1 > unknown > none`.

#### Scenario: Declaration exposes sortable hints without endpoints
- **WHEN** a node reads an `approve_member` declaration
- **THEN** it can obtain `v4_hint` and `v6_hint` values for candidate ordering
- **AND** those hint values do not expose IP addresses or ports

### Requirement: presence state includes governance and decls head summaries
Presence evidence used for MNT-03 SHALL include state-head summaries for governance and decls.

If a receiver observes divergent state-head summaries from a peer, it SHALL be able to trigger best-effort state synchronization or report the divergence as recovery evidence.

#### Scenario: Presence detects state divergence
- **WHEN** a node receives presence with different governance or decls head summaries
- **THEN** it records the divergence
- **AND** topology or recovery diagnostics can report the observed divergence
