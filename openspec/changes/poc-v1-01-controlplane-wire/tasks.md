## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [ ] 1.2 Run baseline `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [ ] 1.3 Run baseline `bash scripts/check_no_xtcp_imports.sh`

## 2. Wire v1 (TLV)

- [ ] 2.1 Add TLV codec helpers (uvarint tag/len) with bounds checks
- [ ] 2.2 Define v1 outer header / inner message field tables and `body_bytes` framing; leave concrete business body schemas to later changes
- [ ] 2.3 Add roundtrip tests for outer header / inner message / ciphertext envelope (golden vectors)

## 3. Transcript v1

- [ ] 3.1 Implement transcript builder with fixed field order and domain-sep
- [ ] 3.2 Add tests: transcript bytes are stable and match golden vectors

## 4. peer_e2e_v1

- [ ] 4.1 Implement sign-then-encrypt (Ed25519 + X25519 + HKDF + XChaCha20-Poly1305)
- [ ] 4.2 Bind AAD to outer header; enforce `msg_id` equality (outer vs inner)
- [ ] 4.3 Add tests: encrypt/decrypt success, tamper fails, expired fails, replay cache hook

## 5. Post-change Validation

- [ ] 5.1 Re-run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [ ] 5.2 Re-run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [ ] 5.3 Re-run `bash scripts/check_no_xtcp_imports.sh`
