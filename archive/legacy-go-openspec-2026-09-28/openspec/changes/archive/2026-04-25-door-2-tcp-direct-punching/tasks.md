# Tasks: door-2-tcp-direct-punching

> Checkbox format requirement: `- [ ] X.Y ...`

## 1) Policy + Wire

- [x] 1.1 Introduce `p2p_network=auto|udp_only|tcp_only` plumbing (types + parsing) and thread it through peer configs (coord + mqtt), `connectivity.Gather`, and `connectivity.Attempt`.
- [x] 1.2 Add CLI flags `-u`/`-t` + `--p2p-network` to **both** `miopunch` POC commands and `miopunch-lab peer`, including YAML config support.
- [x] 1.3 Extend wire messages to carry `capabilities` + `p2p_network` in `PeerHello` and `NatHoleVisitor/NatHoleClient`; keep backward compatibility and update wire roundtrip tests.
- [x] 1.4 Implement `tcp_only` fail-fast during exchange when either peer lacks `tcp_p2p_v0` (applies to coordinator analysis and MQTT `AnalyzeOnce` path).

## 2) Gather (TCP) + Helpers

- [x] 2.1 Implement Door-2 TCP port convention: base `P` for TCP STUN bind and `L=P+100` for TCP listen/punching; fail-fast on invalid/unavailable pinned ports.
- [x] 2.2 Extend `connectivity.Gather` to produce `tcp_direct_addrs` (tcp6/tcp4), keep a stable TCP listener, and include TCP portmap candidates (UPnP/NAT-PMP) for `L`.
- [x] 2.3 Add TCP STUN discovery (explicit + internal cn/global sampling) with endpoint scheme filtering (`tcp://`/`udp://`/dual), binding local TCP source port to `P`, and emitting explainable diagnostics.

## 3) Coordinator TCP Analysis

- [x] 3.1 Derive `tcp_candidate_addrs` and apply the `+100` offset only in coordinator outputs (attempt treats outputs as absolute ports).
- [x] 3.2 Implement TCP punching enablement + error attribution (`tcp_punching_enabled/tcp_punching_error`) and select `tcp_detect_behavior` (mode0..4) with mode2/4 spraying guardrails.

## 4) Attempt (TCP-first) + Punching Kernel

- [x] 4.1 Refactor `connectivity.Attempt` to follow policy order `tcp6 → tcp4 → udp6 → udp4` (or restricted subsets), and for each network run `direct` then `punching`.
- [x] 4.2 Implement TCP direct attempt for `direct_tcp6|direct_tcp4` with observable begin/end events and stable path strings.
- [x] 4.3 Implement TCP punching (simultaneous-open) for `punching_tcp4`, including mode2/4 controlled spraying (`SendRandomPorts`/`ListenRandomPorts`) with bounded `MaxConcurrency/TotalBudget/DialTimeout/SettleWindow` and early-stop on winner.
- [x] 4.4 Update attempt result types to support returning either a UDP path or a TCP stream-ready connection, without breaking existing UDP callers.

## 5) TCP Data Plane (TLS stream)

- [x] 5.1 Add a TCP data plane mode: `tls` (TLS 1.3 stream) used when the selected connectivity path is TCP; keep UDP data plane behavior unchanged.
- [x] 5.2 Implement session-pinned mTLS identity: `HKDF(secret_key, sid, role)` → ed25519 cert key; mutual verification; visitor acts as TLS client and client acts as TLS server.
- [x] 5.3 Implement winner convergence after TLS pinning (visitor leader, client follower) and close non-winner connections.
- [x] 5.4 Plumb TCP data plane through **POC product chain** (`miopunch ping/sh*`) and lab peer paths; ensure `transport.payload_exchanged` evidence remains asserted.

## 6) Lab + STUN server

- [x] 6.1 Extend `stun/server.go` to serve STUN over TCP in addition to UDP (same host:port), and update tests accordingly.
- [x] 6.2 Update `lab/guest/bin/mlab-xtcp-run` to support `p2p_network` flags and new TCP attempt paths; force legacy UDP cases to pass `-u` to preserve regression stability.
- [x] 6.3 Add at least one NAT lab case covering TCP mode2/4 spraying and an ordered events expectation requiring `transport.payload_exchanged`.

## 7) Tests + Validation

- [x] 7.1 Add/extend unit tests for: policy parsing/ordering, `tcp_only` fail-fast, `+100` port convention, TCP STUN discovery wiring, and TLS pinned identity verification.
- [x] 7.2 Run focused validation: `go test ./...` + `go vet ./...` + `bash scripts/check_no_xtcp_imports.sh`.
