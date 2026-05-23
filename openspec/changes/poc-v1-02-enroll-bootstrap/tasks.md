## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [ ] 1.2 Run baseline `export PATH=/usr/local/go/bin:$PATH && go vet ./...`

## 2. InviteCapability (MPINV1)

- [ ] 2.1 Define InviteCapability struct (v1 fixed fields) + TLV encode/decode
- [ ] 2.2 Implement `MPINV1-<base64url>` parsing/formatting
- [ ] 2.3 Unit tests: golden invite vectors

## 3. Join/Approve/Enroll

- [ ] 3.1 Implement join_request publish to join_topic (peer_e2e_v1 to authority)
- [ ] 3.2 Implement approve: verify PoP + issue MemberCredential
- [ ] 3.3 Implement enroll_response publish to reply_topic (peer_e2e_v1 to joiner)

## 4. Persistence

- [ ] 4.1 Persist `MemberCredential + mailbox_secret + broker config` using the v1 persistence layout (defined/implemented by `poc-v1-06-persistence`)

## 5. Post-change Validation

- [ ] 5.1 Re-run `go test ./...` / `go vet ./...`
