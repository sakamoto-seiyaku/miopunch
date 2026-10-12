## Standards — final socket-binding correction

Hard documented-standard breaches: **none found**. Heuristic smells warranting changes: **none**.

Read-only follow-up against base `5beb2c791a929e5343576dfc3e26d7a831e2087f`, restricted to the same 27 owned files. The initial frozen patch and report remain intact. Five owned files differ from their initial-review hashes; the other 22 are unchanged. Protected WIP remains excluded.

The correction implements ADR 0011's explicit rule: “一个保留的 QUIC PathId 保持其本地 socket 绑定；收到另一个 socket 的报文不能接管该路径”. `vendor/noq-proto/src/connection/mod.rs:2505` rejects differing ingress sockets unconditionally for an existing path; the call at line 2374 precedes packet decoding, ACK/frame processing and packet-number deduplication. Original peer-address and local-IP migration behavior is restored. Consequently, iroh's retained path/socket metadata cannot become stale through receive-driven socket takeover.

`vendor/noq-proto/src/tests/multipath.rs:478` verifies that the wrong socket cannot transfer the path or consume the packet, then accepts the identical ciphertext on the correct socket. The separate peer-address migration case at line 544 checks generation invalidation while retaining the local socket. The existing two-socket test continues to require separate PathIds and socket-aware transmit metadata.

ADR 0011 and the fork note consistently describe the corrected binding rule. The implementation note preserves the initial 434-test result as historical evidence and explicitly leaves final demo validation pending. Its final validation paragraph at `docs/notes/2026-10-12-native-socket-api-and-demo.md:25` still needs the planned receipt update: it currently repeats 434 and “接收绑定变化”; use the final suite count and rejection behavior when that evidence is recorded. This is pending note maintenance, not a finding of false completion.

No Cargo invocation or source/test/manifest edits. This follow-up supersedes the initial report's receive-migration interpretation; final SDK/demo acceptance remains outside this report.
