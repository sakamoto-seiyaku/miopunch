## 1. OpenSpec And Contracts

- [x] 1.1 Create proposal, design, delta spec, and tasks for the UDP-only restoration.
- [x] 1.2 Update `dial_offer` / `dial_answer` codec tests for UDP snapshot and decision fields.

## 2. UDP Snapshot And Exchange

- [x] 2.1 Add POC v1 UDP snapshot types that map to legacy `NatHoleVisitor` / `NatHoleClient` decision inputs.
- [x] 2.2 Extend `DialOffer` and `DialAnswer` encode/decode and verification to carry snapshots and answer-side decision outputs.
- [x] 2.3 Replace runtime host-only local candidate assembly with UDP snapshot gathering for the runtime-owned socket.

## 3. Decision And Attempt Path

- [x] 3.1 Generate responder-side decision outputs with `punchdecision.AnalyzeWithDaemonMemory`.
- [x] 3.2 Rewire POC v1 `runPunch` to consume the assigned `NatHoleResp` and call `connectivity.Attempt` with UDP-only policy.
- [x] 3.3 Report UDP punching success back to the daemon analyzer when the selected path is `punching_ipv4`.

## 4. UDP Random Listen

- [x] 4.1 Restore executable UDP `ListenRandomPorts` behavior in `internal/punching.MakeHole`.
- [x] 4.2 Add cleanup and winner ownership tests for temporary random UDP listen sockets.

## 5. Validation

- [x] 5.1 Add focused tests for direct success, direct timeout fallback, decision-driven mode behavior, and codec/verify failures.
- [x] 5.2 Run focused package tests, then run required full host gates; record any lab gate not run.

Validation note:
- `openspec validate restore-pocv1-udp-xtcp-decision --strict` passed after the Must/Should fix follow-up.
- `go test ./internal/pocv1/punch ./internal/punching ./connectivity` passed.
- `go test ./connectivity ./internal/pocv1/punch ./internal/pocv1/persist ./internal/pocv1/enroll ./internal/pocv1/runtime` passed after the Must/Should fix follow-up.
- `go test ./...` passed.
- `go vet ./...` passed.
- `bash scripts/check_no_xtcp_imports.sh` passed.
- Lab gates were attempted. Default `LAB_SSH_PORT=2222` failed because the host forward port was in use; retry with `LAB_SSH_PORT=2223` started QEMU without KVM acceleration, but SSH was not ready after 120 seconds. The VM was cleaned up with `LAB_SSH_PORT=2223 ./lab/host/labctl down`.
