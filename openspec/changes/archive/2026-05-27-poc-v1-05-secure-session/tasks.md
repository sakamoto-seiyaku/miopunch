## 1. Session Recipe

- [x] 1.1 Add `internal/pocv1/session` and define `SessionRecipe`, `PeerSession`, `OpenStream`, and `AcceptStream`.
- [x] 1.2 Implement the single v1 recipe: `UDP + KCP + TLS1.3 + yamux`.
- [x] 1.3 Accept only `PathResult` as input; remote `peer_id` and remote `MemberCredential` must already be carried inside it, and the session layer must not re-open punch selection or roster lookup here.

## 2. Identity Pinning

- [x] 2.1 Implement 6A pinning so the peer certificate Ed25519 public key must match `MemberCredential.subject_ed25519_pub`.
- [x] 2.2 Verify the remote credential against the network authority before accepting the session.
- [x] 2.3 Reuse legacy `dataplane` / `tlsutil` only behind the new v1 recipe adapter; do not keep the legacy `secretKey + sid + role` pin contract as the 05 source of truth.

## 3. Acceptance

- [x] 3.1 Add tests for successful `PathResult -> PeerSession` upgrade, `OpenStream/AcceptStream` behavior, and stream-open metadata preservation.
- [x] 3.2 Add tests for pin mismatch, invalid credential, and handshake failure.
