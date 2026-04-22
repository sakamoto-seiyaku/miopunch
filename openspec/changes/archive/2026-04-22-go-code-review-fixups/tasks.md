## 1. Pre-flight Checks

- [x] 1.1 Run `gofmt -l .` and ensure clean
- [x] 1.2 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 1.3 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 1.4 Run `bash scripts/check_no_xtcp_imports.sh`

## 2. `internal/control` (Control Plane)

- [x] 2.1 Fix `quicStreamRWC.Close()` to close both stream and underlying QUIC conn (no leaks)
- [x] 2.2 Rename control QUIC ALPN constant to `miopunch-*` naming (remove `xtcp` residue)
- [x] 2.3 Add a minimal regression test for QUIC dial/accept + close behavior (loopback)

## 3. `internal/dataplane/congestion/bbr` (BBR)

- [x] 3.1 Replace `fmt.Printf` stdout debug output with repo logger (`internal/logutil`) at debug level
- [x] 3.2 Ensure debug prints do not break event JSON output (no stdout pollution from libraries)

## 4. `dataplane/` (Payload Exchange)

- [x] 4.1 Remove dead helper `withDeadline` (unused + misleading)
- [x] 4.2 Handle/propagate errors from deadline setters (`SetDeadline`) instead of discarding
- [x] 4.3 Rename dataplane QUIC ALPN constant to `miopunch-*` naming (remove `xtcp` residue)

## 5. `internal/peer` (Peer Orchestration)

- [x] 5.1 Remove unused frame helpers in `internal/peer/session.go` (avoid duplicated, unused code)
- [x] 5.2 Remove unused NAT hole prepare glue in `internal/peer/prepare.go` (or re-home if still needed)
- [x] 5.3 Reduce timer leaks in long-lived selects (prefer `time.NewTimer` over `time.After` where appropriate)

## 6. `connectivity/` (Docs & Naming)

- [x] 6.1 Update `connectivity` package doc to use `miopunch` naming (avoid `xtcp` in current semantics)

## 7. Post-change Validation

- [x] 7.1 Re-run: `gofmt -l .`, `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`
- [x] 7.2 If runtime behavior is touched, run relevant lab selftests (`./lab/host/labctl ...`) before mainline merge
