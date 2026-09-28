## 1. Parallel Extraction Setup

- [x] 1.1 Add `internal/pocv1/wire` and `internal/pocv1/peere2e` as the only new owners for current v1 peer-targeted wire/security logic.
- [x] 1.2 Treat legacy `internal/controlplane` message/sign/encoding code as read-only reference for v1 extraction; do not add new v1 semantics there.

## 2. TLV + Envelope Contract

- [x] 2.1 Implement canonical TLV encode/decode helpers for `bytes/u64/ascii` plus strict reject rules for unknown tags, duplicates, non-canonical uvarints, truncation, and out-of-order fields.
- [x] 2.2 Implement v1 outer relay header and inner peer message structs plus deterministic encoder/decoder.
- [x] 2.3 Enforce outer/inner invariants: matching `msg_id`, matching `expires_at`, fixed `scheme=peer_e2e_v1`, fixed-size ids/keys/signatures, and the four-name `kind` allowlist.

## 3. Transcript + peer_e2e_v1

- [x] 3.1 Implement the fixed transcript builder (`domain-sep + TLV(fields...)`) and direct Ed25519 signing with no JSON canonicalization and no pre-hash.
- [x] 3.2 Implement `peer_e2e_v1` seal/open using X25519 + HKDF-SHA256 + XChaCha20-Poly1305 with AAD bound only to trusted outer fields.
- [x] 3.3 Implement ciphertext frame parsing/encoding for `"MP1" || eph_pub || nonce24 || aead_ct`.

## 4. Handoffs + Acceptance

- [x] 4.1 Expose only top-level `kind` names from 01; keep body parsing/validation delegated to `02` and `04`.
- [x] 4.2 Add golden vectors for outer/inner/transcript/ciphertext and byte-for-byte tests for deterministic fixtures.
- [x] 4.3 Add tamper/expiry/replay-hook tests proving invalid messages are dropped without network error replies.
- [x] 4.4 Prove the current v1 runtime path no longer relies on the legacy POC v0 JSON/AES-GCM wire contract as its source of truth.
- [x] 4.5 Add one remote MQTT round-trip integration test using a broker selected from the legacy built-in list; skip only for network or broker availability failures, and fail for non-network logic regressions.
