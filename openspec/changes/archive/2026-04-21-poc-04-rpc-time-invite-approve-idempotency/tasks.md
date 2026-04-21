## 1. RPC time semantics (receiver)

- [x] 1.1 Add RPC request classifier (`kind` suffix `_request`)
- [x] 1.2 Implement receiver-side time validation: `expires_at_unix_ms` required for RPC requests + strict expiry drop
- [x] 1.3 Implement receiver-side sanity drop: `abs(now-created_at)>10m` with a surfaced clock-skew diagnostic
- [x] 1.4 Unit tests for expiry/clock-skew validation and diagnostics

## 2. Idempotency cache (handled requests)

- [x] 2.1 Implement in-memory `handled_requests` cache: `request_msg_id -> cached_response_ciphertext` with TTL (min 10m) + cap (suggest 1024)
- [x] 2.2 Enforce retry invariants for the same `request_msg_id` (only time bounds may change; other transcript fields must match)
- [x] 2.3 Unit tests for cache hit/miss, TTL eviction, and transcript mismatch handling

## 3. Dedup boundary (forwarding vs dst=self RPC)

- [x] 3.1 Refactor `internal/controlplane` inbound flow so `dst=self` RPC requests are not dropped solely due to dedup (forwarding path remains dedup-dropped)
- [x] 3.2 Ensure duplicate `dst=self` RPC requests trigger idempotent response re-send and do not re-apply side effects
- [x] 3.3 Integration tests covering: forwarded duplicates drop; `dst=self` duplicate RPC request reaches idempotency path

## 4. Invite store persistence (issuer/admin)

- [x] 4.1 Implement `invite_id = base32(raw,no-pad, sha256(invite_topic)[:16])` helper for stable indexing
- [x] 4.2 Define and implement `invites/<invite_id>.json` read/write (tmp→fsync→rename) with `uses_left` + `handled_requests{request_msg_id: response_ct_b64url}`
- [x] 4.3 Implement atomic `uses_left` decrement-at-most-once per `request_msg_id` with a process-local lock
- [x] 4.4 Unit tests for persistence format, restart recovery, and duplicate request does-not-decrement

## 5. Invite/approve handler integration

- [x] 5.1 Wire invite/approve request handling to persistent store: handled hit → re-send cached response; miss → validate time/uses, apply once, store response
- [x] 5.2 Integration test: issuer restart + replay same `request_msg_id` does not consume additional `uses_left` and re-sends cached response

## 6. Smoke harness update (optional but recommended)

- [x] 6.1 Update `tools/miopunch-cp-smoke` to model RPC request/response (`*_request` + `in_reply_to`) and include `expires_at_unix_ms`
- [x] 6.2 Add smoke mode to replay the same `request_msg_id` and assert idempotent cached response behavior

## 7. Validation

- [x] 7.1 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 7.2 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 7.3 Run `bash scripts/check_no_xtcp_imports.sh`
