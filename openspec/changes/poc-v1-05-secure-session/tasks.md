## Done

- Freeze the v1 `SessionRecipe`: `PathResult` in, `PeerSession` out, with a single `UDP + KCP + TLS1.3 + yamux` path.
- Freeze 6A TLS pinning against `MemberCredential.subject_ed25519_pub` plus authority verification.
- Keep upper layers on `PeerSession/OpenStream` only; do not reintroduce path-selection logic here.
