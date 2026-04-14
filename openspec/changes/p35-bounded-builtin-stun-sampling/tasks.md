## 1. Change Setup

- [x] 1.1 Reorder built-in `cn` and `global` STUN lists so verified stable endpoints appear first
- [x] 1.2 Keep source annotations in code comments for newly added built-in STUN endpoints

## 2. Bounded-Concurrency Internal STUN Sampling

- [x] 2.1 Refactor internal STUN discovery so built-in sampling can use a single UDP socket with transaction-based response dispatch
- [x] 2.2 Add bounded-concurrency sampling for one STUN view with per-view stop conditions and cancellation
- [x] 2.3 Run `cn` and `global` built-in STUN views concurrently within the existing gather timeout budget
- [x] 2.4 Keep explicit `--stun` / `stun:` discovery behavior unchanged

## 3. Regression Coverage

- [x] 3.1 Add focused tests for per-view early-stop behavior
- [x] 3.2 Add focused tests for concurrent response dispatch on a shared UDP socket
- [x] 3.3 Update built-in STUN list tests to match the reordered prioritized defaults

## 4. Validation

- [x] 4.1 Run `go test ./connectivity ./internal/coordinator ./internal/punching`
- [x] 4.2 Rebuild host + Android binaries
- [x] 4.3 Re-run Android mobile-network `case1` using only built-in STUN and capture the result in `docs/reports`
