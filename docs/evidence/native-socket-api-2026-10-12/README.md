# Native local socket API — MIOPUNCH-90

Final source graph `48d1499a7e204d5240f6ec911ac891caae8aa2f4e066f2086c31a760fb7eb598`.
See [ADR 0011](../../adr/0011-native-local-socket-identity.md) and
[implementation note](../../notes/2026-10-12-native-socket-api-and-demo.md).

Noq 435 + affected SDK 81 tests pass. Eleven affected cases use actual physical
sockets, including original-session streams, source pinning, ownership, TTL,
retirement and cancellation. New protocol cases distinguish same-address
socket paths, reject wrong-socket authenticated packets without consuming their
packet numbers, and invalidate old probes after peer-address migration. These
protocol cases use synthetic I/O metadata; no physical forwarding-proxy case is claimed.

The current Native/WASM demo passes on owned localhost Ergo TLS/WSS and an
owned HTTP relay: real restricted request, one approval record, durable material
save, both identities reopened, member1 stream PING/PONG on the original admitted
connection, natural SDK and service cleanup. End-to-end QUIC/TLS remains actual;
outer relay HTTP and private test-CA trust are controlled-fixture qualifications.
Caller/attempt60s, pacing10ms, assembly8s and queue30s are explicit fixture
settings. SAFERATE=false is observed, not inferred. This is no full-default,
public-network or physical-platform matrix certificate. Some probe phase labels
are inherited from its public predecessor; executed destinations are loopback.

The proposed public run was rejected by automatic approval review before any
actor started, because the visible record lacked authorization for its specific
sensitive payload and public destination. It was not retried indirectly. The
first controlled run failed before sending its request because an inherited
performance assertion required SAFERATE; its original failure is retained.
The second changes the controller readiness assertion and owned pacing setting,
using identical SDK artifacts; all functional and cleanup checks pass.

Initial Spec review found a stale socket cache risk from receive-driven socket
migration. Final retained PathIds keep their local socket binding; changing it
requires another explicit path on the same M. Final two-axis review has zero
open findings. The first mutable implementation and its passing tests remain
historical, not final qualification.

All builds use the shared repository target and preserved locked graphs.
Final Native build:529 Fresh/0 rebuilt; WASM:291/5; affected tests:521/20;
Noq:145/1. Immutable pins, actual selected inputs and raw logs are retained.
Only reviewed historical incremental IR was cleaned:20,682,448,896 bytes.
After independent host-process/container checks, four generated private keys
and two private NSS databases were removed; raw evidence and SDK pins remain.

Files `*-build.json`, `*-build-context.json`, and `*-selected-inputs.json`
bind the actual artifacts to source, locks and tools. `controlled-demo.json`
is a scalar projection; its full private original is bound by SHA256.
The exact selectors/controllers retain their executed local paths and require
the retained private fixture setup and pinned baseline archives for replay.
