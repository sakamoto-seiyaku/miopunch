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

### Requirement: snapshot_body hash is canonical and stable
For POC v0, the governance snapshot head SHALL be represented by `snapshot_body` plus `signatures`.

The snapshot body hash SHALL be computed as:

- `hash_b64 = base64url(no-pad, sha256(canonical_json(snapshot_body)))`

`hash_b64` is for display and state alignment; receivers MUST recompute and verify it.

### Requirement: signatures cover snapshot_body hash
For POC v0, each signature entry SHALL be:

- `signature = { key_id, sig_b64 }`

Where:

- `key_id = hex(sha256(ed25519_pub))` (TUF-style key identifier; 64 hex chars)
- `sig_b64 = base64url(no-pad, Ed25519.Sign(owner_priv, sha256(canonical_json(snapshot_body))))`

### Requirement: head snapshot is persisted on disk
Nodes SHALL persist the current governance head snapshot at:

- `governance/head_snapshot.json` under the node state directory.

### Requirement: receiver validates updates with old+new thresholds
When applying a candidate snapshot and a local head snapshot exists, the node SHALL:

- reject updates whose `prev_hash_b64` does not match the local head hash (to avoid forks in POC v0)
- require `height == local_height+1`
- require `net_id == local_net_id`
- require **old-threshold (=1)**: at least one signature verifiable under the local head owners (old trust root)
- require **new-threshold (=1)**: at least one signature verifiable under the candidate owners (self-signed new trust root)

If the candidate hash equals the local head hash, the node MAY treat it as a no-op success.

### Requirement: bootstrap acceptance uses self-signed snapshot
When accepting an initial snapshot and no local head snapshot exists (e.g., join bootstrap), the node SHALL validate the snapshot as self-contained:

- recompute and verify `hash_b64`
- require **new-threshold (=1)**: at least one signature verifiable under the candidate owners
- require `net_id == local_net_id` (derived from `net_secret`)
