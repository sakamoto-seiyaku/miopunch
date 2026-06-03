## 1. POC v1 UDP Direct Path

- [x] 1.1 Add `direct_ipv4` path evidence to current v1 punch attempt and selected path result types without changing the `dial_offer` / `dial_answer` body.
- [x] 1.2 Implement host-to-host IPv4 UDP direct handshake before `punching.MakeHole` in `internal/pocv1/punch`, using the existing shared UDP socket and traversal demux.
- [x] 1.3 Keep existing UDP punching behavior as fallback when direct UDP times out, is not applicable, or fails.
- [x] 1.4 Preserve existing mirrored-host shortcut behavior while ensuring Android/WSL different-IP same-LAN candidates can use direct UDP.

## 2. Evidence and Diagnostics

- [x] 2.1 Include `selected_path=direct_ipv4|punching_ipv4` in successful current v1 ping/session evidence and report data.
- [x] 2.2 Include direct-attempt timeout/failure evidence in Punch-stage failures before punching fallback facts.
- [x] 2.3 Wire direct/punch attempt trace events into daemon logs when `miopunch up --log-level trace` is used.
- [x] 2.4 Add trace-level traversal demux diagnostics for receive, decode failure, unknown transaction, endpoint routing, queue-full drop, and best-effort auto-response decisions.

## 3. Documentation and Specs

- [x] 3.1 Update current POC v1 dial/punch documentation to describe UDP direct-first with UDP punching fallback.
- [x] 3.2 Update Android control-client note with the restored demo path and the explicit non-goal of TCP Door-2 restoration in this change.
- [x] 3.3 Keep `ping -t` / `p2p_network=tcp_only` documented as follow-up scope, not acceptance scope for this change.

## 4. Automated Validation

- [x] 4.1 Add unit tests for host-to-host UDP direct success in `internal/pocv1/punch`.
- [x] 4.2 Add unit tests for direct timeout followed by UDP punching fallback.
- [x] 4.3 Add tests that selected path evidence is surfaced through current v1 runtime/CLI output.
- [x] 4.4 Run `export PATH=/usr/local/go/bin:$PATH && go test ./internal/pocv1/punch ./internal/pocv1/runtime ./cmd/miopunch -count=1`.

## 5. Real Environment Validation

- [x] 5.1 Build Linux and Android arm64 `cmd/miopunch` binaries from the changed tree.
- [x] 5.2 Run WSL/Linux and Android daemons with `--log-level trace` and a controlled broker.
- [x] 5.3 Verify `init-network -> invite -> join -> approve -> ls -> ping` succeeds between WSL and Android and reports `selected_path=direct_ipv4`.
- [x] 5.4 Verify `sh ls` succeeds for the Android/WSL demo path.
- [x] 5.5 Capture the relevant CLI JSON reports and both daemon logs as implementation evidence.

## 6. Review Fixes

- [x] 6.1 Preserve `selected_path` facts on current v1 `ping`, `sh ls`, and `sh` failure paths after a peer session is established.
- [x] 6.2 Preserve selected path and selected endpoint facts when punch succeeds but secure-session upgrade fails.
- [x] 6.3 Add focused unit tests for selected-path failure evidence and direct/punch attempt failure diagnostics.
