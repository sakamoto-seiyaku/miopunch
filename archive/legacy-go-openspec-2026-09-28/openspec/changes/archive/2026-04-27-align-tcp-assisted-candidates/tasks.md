## 1. Wire And Gather

- [x] 1.1 Add `tcp_assisted_addrs` to request/response wire messages and roundtrip tests.
- [x] 1.2 Update TCP gather to classify true direct candidates separately from assisted/private listen addresses.
- [x] 1.3 Respect `DisableAssistedAddrs` for both UDP assisted and TCP assisted exchange.
- [x] 1.4 Reject or diagnose obvious private IPv4 addresses that still appear in `tcp_direct_addrs`.

## 2. Decision And Attempt

- [x] 2.1 Update punching decision to preserve direct, assisted exact, candidate exact, and candidate expanded TCP buckets.
- [x] 2.2 Allow minimal assisted-only mode0 fallback when TCP STUN evidence is insufficient but assisted targets exist.
- [x] 2.3 Update `direct_tcp4` to consume only peer TCP direct addresses.
- [x] 2.4 Update `punching_tcp4` target building so assisted exact targets are not range/random expanded.
- [x] 2.5 Add diagnostics for assisted exact count, candidate exact count, candidate expanded count, and winning target source.

## 3. Tests

- [x] 3.1 Add unit tests for TCP address classification and direct/assisted target building.
- [x] 3.2 Update F-002 MNT-01 cases so private-address fallback is asserted as `punching_tcp4`.
- [x] 3.3 Add or defer true TCP4 direct/portmap direct coverage until fixtures produce real direct candidates.

## 4. Verification

- [x] 4.1 Run focused TCP gather/decision/attempt tests.
- [x] 4.2 Run `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [x] 4.3 Run MNT-01 TCP related smoke/selftest cases.
