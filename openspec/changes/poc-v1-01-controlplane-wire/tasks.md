## 1. TLV v1 Primitives

- [ ] 1.1 Implement TLV encode helpers: canonical uvarint tag/len, bytes/u64/ascii field writers.
- [ ] 1.2 Implement TLV decode with bounds checks and non-canonical uvarint detection (reject).
- [ ] 1.3 Implement strict field rules: increasing-tag order, no-duplicates, and unknown-tag rejection for a declared tag table.
- [ ] 1.4 Add unit tests for canonical encoding and rejection cases (dup/unknown/non-canonical/truncation/len mismatch).

## 2. v1 Outer/Inner Envelope

- [ ] 2.1 Implement v1 outer relay header encode/decode per design tag table (`v=1`, `scheme=peer_e2e_v1`).
- [ ] 2.2 Implement v1 inner peer message encode/decode per design tag table (including optional `in_reply_to`).
- [ ] 2.3 Enforce invariants: outer/inner `msg_id` match, outer/inner `expires_at` match, kind allowlist, and fixed-length fields (16/32/64 bytes).
- [ ] 2.4 Add roundtrip + determinism tests for outer/inner TLV bytes (golden vectors for fixed inputs).

## 3. v1 Transcript

- [ ] 3.1 Implement transcript builder (`domain-sep + TLV(fields...)`) with the fixed transcript field order.
- [ ] 3.2 Add tests that transcript bytes are stable and match golden vectors.

## 4. peer_e2e_v1 (Sign-Then-Encrypt)

- [ ] 4.1 Implement sign step: Ed25519 signs transcript bytes directly (no pre-hash) and writes `sig_ed25519` into inner.
- [ ] 4.2 Implement encrypt/decrypt: X25519 + HKDF-SHA256 + XChaCha20-Poly1305 with AAD bound to outer header (excluding `ct` and excluding untrusted `src`).
- [ ] 4.3 Implement ciphertext frame parsing/encoding: `"MP1" || eph_pub || nonce24 || aead_ct`.
- [ ] 4.4 Add tests: decrypt success, tamper fails (ct/AAD), expired fails, replay hook contract.

## 5. Error Semantics + Evidence Hooks

- [ ] 5.1 Implement drop-only error handling with reason aggregation categories (`decrypt_fail/bad_sig/expired/replay/unsupported_version/malformed`).
- [ ] 5.2 Add tests that invalid messages are dropped and only evidence/aggregation is updated.
