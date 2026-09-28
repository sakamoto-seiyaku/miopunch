## 1. Dependencies

- [x] 1.1 Add `yamux` dependency (with `replace` to FRP pinned fork) and remove `smux` from `go.mod` / `go.sum`

## 2. Dataplane Mux Implementation

- [x] 2.1 Replace `smuxPeerSession` with `yamuxPeerSession` for TCP/TLS sessions
- [x] 2.2 Replace `smuxPeerSession` with `yamuxPeerSession` for KCP sessions
- [x] 2.3 Accept inbound streams using `AcceptStreamWithContext(ctx)` (remove session-level deadline polling)
- [x] 2.4 Configure yamux to avoid stderr log noise (`LogOutput=io.Discard`) and keep behavior aligned with FRP where appropriate
- [x] 2.5 Run `gofmt` on touched Go files

## 3. Observability

- [x] 3.1 Emit logical stream close diagnostics as `info` always, and attach `kvs.close_err` when `Close()` returns an error

## 4. Tests

- [x] 4.1 Add unit test that asserts stream close event is `info` even when close returns error, and includes `close_err`
- [x] 4.2 Ensure existing TLS/KCP session tests still validate “stream close does not close session”

## 5. Validation

- [x] 5.1 Run host gates: `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`
- [x] 5.2 Run full lab gates: `./lab/host/labctl selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`

## 6. Verification

- [x] 6.1 Run `openspec validate switch-smux-to-yamux --type change`
