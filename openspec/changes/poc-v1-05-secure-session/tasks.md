## 1. Session Recipe

- [ ] 1.1 Add `internal/pocv1/session` and define `SessionRecipe`, `PeerSession`, `OpenStream`, and `AcceptStream`.
- [ ] 1.2 Implement the single v1 recipe: `UDP + KCP + TLS1.3 + yamux`.
- [ ] 1.3 Accept only `PathResult` as input; remote `peer_id` and remote `MemberCredential` must already be carried inside it, and the session layer must not re-open punch selection or roster lookup here.

## 2. Identity Pinning

- [ ] 2.1 Implement 6A pinning so the peer certificate Ed25519 public key must match `MemberCredential.subject_ed25519_pub`.
- [ ] 2.2 Verify the remote credential against the network authority before accepting the session.
- [ ] 2.3 Reuse legacy `dataplane` / `tlsutil` only behind the new v1 recipe adapter.

## 3. Acceptance

- [ ] 3.1 Add tests for successful `PathResult -> PeerSession` upgrade and `OpenStream/AcceptStream` behavior.
- [ ] 3.2 Add tests for pin mismatch, invalid credential, and handshake failure.
