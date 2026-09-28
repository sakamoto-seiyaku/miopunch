## 1. Phase Plan And Executor

- [x] 1.1 Define an internal phase plan shape covering role, delay, budget, targets, TTL behavior, and diagnostic labels.
- [x] 1.2 Refactor UDP punching attempt to start receive loop before probe loop.
- [x] 1.3 Add bounded UDP probe loop behavior that repeats within budget and cancels on success.
- [x] 1.4 Align TCP attempt diagnostics with the same phase plan vocabulary without regressing current TCP receive-first behavior.

## 2. Analyzer Memory

- [x] 2.1 Add daemon-lifetime success-only analyzer storage for MQTT/task paths.
- [x] 2.2 Scope analyzer keys by remote peer, protocol, and existing NAT analysis key.
- [x] 2.3 Record only successful mode/index state with bounded TTL; do not record failures or endpoints.

## 3. Diagnostics And Tests

- [x] 3.1 Emit receive/probe lifecycle diagnostics for UDP and TCP punching.
- [x] 3.2 Add focused tests for phase plan execution ordering and cancellation.
- [x] 3.3 Update MNT-01 UDP NAT2/NAT3 expectations to require success or stronger phase diagnostics.
- [x] 3.4 Fix the F-004 IPv6-to-UDP4 fallback case expectation or fixture setup so `direct_ipv4` is only required when a true direct candidate exists.

## 4. Verification

- [x] 4.1 Run `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [x] 4.2 Run `./lab/host/labctl mnt01-smoke` and `./lab/host/labctl mnt01-selftest`.
- [x] 4.3 Run required full gates before mainline merge.
