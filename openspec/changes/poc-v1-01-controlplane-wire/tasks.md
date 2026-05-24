## Done

- Freeze v1 peer-targeted control-plane envelope: outer relay header + inner peer message + `body_bytes` framing.
- Freeze v1 TLV wire with canonical encoding + strict reject rules (unknown/dup/non-canonical/out-of-order).
- Freeze v1 transcript construction (`domain-sep + TLV(fields...)`) with fixed field order and no JSON canonicalization.
- Freeze `peer_e2e_v1` as sign-then-encrypt recipient-only (Ed25519 + X25519 + HKDF-SHA256 + XChaCha20-Poly1305) with AAD bound to outer header.
- Freeze v1 message kind allowlist: `join_request`, `enroll_response`, `dial_offer`, `dial_answer`.
- Freeze golden vectors (hex fixtures) for outer/inner/ciphertext/transcript determinism checks.
