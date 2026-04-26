## 1. Session Abstractions

- [ ] 1.1 Add peer transport session interfaces/types for opening logical streams and closing sessions with reasons.
- [ ] 1.2 Add an in-memory per-peer session manager owned by the daemon/task runtime.
- [ ] 1.3 Define session keys using remote peer identity, transport protocol, security identity, and path family.
- [ ] 1.4 Add keepalive/idle timeout hooks and close reason diagnostics.

## 2. Transport Implementations

- [ ] 2.1 Implement TCP session as pinned TLS 1.3 plus smux.
- [ ] 2.2 Implement KCP session as KCP over UDP plus pinned TLS 1.3 plus smux.
- [ ] 2.3 Update QUIC session to expose native logical streams and proper identity-binding diagnostics.
- [ ] 2.4 Ensure logical stream close does not close the underlying session.

## 3. Stream Open And Shell Transition

- [ ] 3.1 Add generic stream-open envelope with kind and metadata.
- [ ] 3.2 Add stream-open authorization using peer membership, revocation, kind, and target/session metadata.
- [ ] 3.3 Adapt current ping/sh paths to run over `shell.v0` logical streams.
- [ ] 3.4 Keep transitional shell hello behavior only where needed for compatibility during this change.

## 4. Tests

- [ ] 4.1 Add unit tests for logical stream close versus session close.
- [ ] 4.2 Add KCP session test covering stream-open/hello/ping on the same established session.
- [ ] 4.3 Add smux-backed TCP/KCP tests for multiple sequential logical streams.
- [ ] 4.4 Tighten `mnt01-smoke-kcp-transport` to require `hello=ok` and `ping=ok`.

## 5. Verification

- [ ] 5.1 Run focused dataplane/session tests.
- [ ] 5.2 Run `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [ ] 5.3 Run `./lab/host/labctl mnt01-smoke` and relevant MNT-01 selftest cases.
- [ ] 5.4 Run required full gates before mainline merge.
