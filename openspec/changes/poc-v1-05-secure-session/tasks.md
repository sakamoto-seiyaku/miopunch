## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`

## 2. KCP + TLS + yamux recipe (v1 only)

- [ ] 2.1 Ensure the recipe code path is single-choice (no QUIC/TCP branching in v1)
- [ ] 2.2 Wire the punched UDP path into KCP session
- [ ] 2.3 Upgrade to TLS1.3 and then to yamux streams

## 3. TLS pin to MemberCredential (6A)

- [ ] 3.1 Generate Ed25519 self-signed cert from device identity key
- [ ] 3.2 Implement VerifyConnection: cert pubkey == credential.subject_ed25519_pub
- [ ] 3.3 Verify MemberCredential with authority key and time window

## 4. Post-change Validation

- [ ] 4.1 Re-run `go test ./...`
