## 1. Pre-flight Validation

- [ ] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`

## 2. dial_offer / dial_answer

- [ ] 2.1 Define v1 dial message bodies + TLV encode/decode
- [ ] 2.2 Implement inbox delivery: offer/answer publish to peer inbox_topic (derived locally from mailbox_secret + peer_id)
- [ ] 2.3 Unit tests: offer/answer roundtrip + E2E encrypt/decrypt

## 3. Punch attempt strategy (5B)

- [ ] 3.1 Implement attempt matrix with max concurrency=4 and total budget=10s
- [ ] 3.2 Evidence capture: candidate table + attempt results

## 4. Post-change Validation

- [ ] 4.1 Re-run `go test ./...`
