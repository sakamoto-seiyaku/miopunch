## 1. Wire format (POC v0)

- [x] 1.1 Add `msg_id` generator + canonicalizer (base32 raw no-pad, 16B → 26 chars)
- [x] 1.2 Add control-plane message structs (`proto_version`, `route`, `signed`) with JSON encode/decode helpers

## 2. Signing (dst covered, hop_limit excluded)

- [x] 2.1 Implement transcript builder and Ed25519 sign/verify helpers
- [x] 2.2 Add receiver-side guard: verify signature then enforce `dst_peer_id == self_peer_id`

## 3. Group wrapper crypto + ciphertext framing

- [x] 3.1 Implement HKDF-derived AES-256-GCM key derivation for group wrapper (`miopunch/v0/aead.ctrl.group`)
- [x] 3.2 Implement `v||nonce||ct` framing (v=0, nonce=12B) seal/open helpers

## 4. Dedup window (LRU + TTL)

- [x] 4.1 Implement `seen` cache (cap=8192, ttl=10m) with a testable clock

## 5. Bounded flooding forwarder (H=3)

- [x] 5.1 Implement forwarding engine: decrypt → hop_limit check → dedup → deliver or forward (hop_limit--) → exclude source neighbor
- [x] 5.2 Implement bounded forward queue (max=1024) + drop stats (`mesh_forward_drops`)
- [x] 5.3 Ensure forwarder worker lifecycle: explicit `Close()` + wait (no goroutine leaks)

## 6. Automated tests

- [x] 6.1 Unit tests: signature coverage (dst vs hop_limit), framing roundtrip/version reject
- [x] 6.2 Unit tests: `seen` TTL + LRU eviction behavior
- [x] 6.3 3-node in-memory integration test: A→B→C forwarding with H=3, dedup, queue drops

## 7. LAN 3-process smoke harness (mesh-first + MQTT fallback)

- [x] 7.1 Add `tools/miopunch-cp-smoke` to run a minimal control-plane node over UDP (mesh) + MQTT inbox subscribe/publish
- [x] 7.2 Add smoke mode to send a request A→C (via B), validate mesh-first then MQTT fallback without duplicate side effects, and print drop facts
- [x] 7.3 Document how to run the 3-process LAN smoke (commands/flags/topology)

## 8. Validation

- [x] 8.1 Run `go test ./...`
- [x] 8.2 Run `go vet ./...`
- [x] 8.3 Run `bash scripts/check_no_xtcp_imports.sh`
