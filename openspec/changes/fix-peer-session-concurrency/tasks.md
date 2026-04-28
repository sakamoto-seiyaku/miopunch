## 1. MQTT Attempt Bucketing (dial_id)

- [ ] 1.1 Introduce `dial_id`-scoped topic helpers for MQTT signaling (`<sid>/attempt/<dial_id>/...`) and keep SID-level `hello/*` barrier unchanged
- [ ] 1.2 Update visitor path to publish `info/visitor` first and use bucketed topics for `info/* resp/* ready/* start`
- [ ] 1.3 Update client path to discover `dial_id` from `info/visitor`, run per-attempt handlers concurrently, and publish all responses within the same bucket
- [ ] 1.4 Add focused unit tests proving two concurrent visitors within the same SID do not stomp (bucket isolation + barrier/start independence)

## 2. Dataplane Server Accept Loop (QUIC/KCP/TLS)

- [ ] 2.1 Introduce a server-side abstraction that can accept multiple inbound peer transport sessions (e.g., a `PeerSessionListener` with `Accept(ctx) (PeerSession, error)` + `Close()`)
- [ ] 2.2 Implement QUIC listener with accept loop (do not single-shot accept-and-close); ensure it can accept multiple inbound connections on the same UDP socket
- [ ] 2.3 Implement KCP listener using `ServeConn` + `Accept/AcceptKCP` (replace fixed `conv=1` single-session binding) and wire it into TLS 1.3 identity binding + yamux
- [ ] 2.4 Implement TLS/TCP listener accept loop wiring for inbound sessions (consistent with QUIC/KCP listener abstraction)
- [ ] 2.5 Ensure accept loops have deterministic shutdown (ctx cancel / daemon shutdown) and emit transport diagnostics on fatal errors

## 3. Acceptor Multi-Session Serve Model

- [ ] 3.1 Refactor acceptor runtime to avoid single-slot `serveOnce -> AcceptStream()` monopolization; move per-session stream serving into per-session goroutines
- [ ] 3.2 Ensure acceptor can handle multiple inbound sessions concurrently (p2 connected does not block p3)
- [ ] 3.3 Keep default behavior: per peer-pair prefer a single “main session” and multiplex operations as logical streams (no per-operation re-punch / re-connect)

## 4. Revoke Strong Semantics (Passive Notification, Active Cut-Off)

- [ ] 4.1 Introduce/extend a session registry keyed by verified `peer_id` that tracks active peer transport sessions
- [ ] 4.2 On observing a valid `revoke_member` tombstone locally, proactively close all active sessions for that `peer_id` with `authorization_revocation` close reason
- [ ] 4.3 Ensure revoked peers are rejected on new stream-open/hello paths, with reports attributing denial to authorization/revoke instead of generic transport failure

## 5. MNT-02 Gate Updates (Remove Workaround, Add Evidence)

- [ ] 5.1 Remove the current MNT-02 selftest workaround that terminates a member daemon to “free the acceptor loop”
- [ ] 5.2 Add/strengthen a required case proving: while `p2` session exists, `p3` can still `ping` (and at least one `sh` op) the same target peer successfully
- [ ] 5.3 Extend revoke coverage to assert: revoked member is denied AND its existing session is cut off; non-revoked member remains able to dial

## 6. Verification

- [ ] 6.1 Run `go test ./...` and `go vet ./...`
- [ ] 6.2 Run `./lab/host/labctl mnt02-smoke` and ensure gate passes with artifacts summary
- [ ] 6.3 Run `./lab/host/labctl mnt02-selftest` and ensure gate passes with artifacts summary
- [ ] 6.4 Run the full mainline gate set: `bash .codex/skills/dev/scripts/run_test_gates.sh`

