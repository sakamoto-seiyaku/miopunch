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

### Requirement: decl signature is verifiable and deterministic
For POC v0, `sig_b64` MUST be computed over a deterministic transcript derived from the decl contents (excluding `sig_b64`).

Nodes MUST reject decls whose signature cannot be verified against the issuer key.

### Requirement: decls converge by set-union
Nodes SHALL store `decls` as a set keyed by `msg_id` (duplicates are ignored).

### Requirement: revoke_member is a permanent tombstone
If a `revoke_member` decl exists for a given member identity, the node SHALL treat that identity as revoked permanently (cannot be re-approved with the same identity key).

### Requirement: decls head hash can be reported
Nodes SHALL be able to report a `decls_head_b64` summary hash computed from the decl set contents to detect divergence and trigger best-effort synchronization.

