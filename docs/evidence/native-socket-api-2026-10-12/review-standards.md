## Standards

Hard documented-standard breaches: **none found** in the frozen owned changes.

Reviewed the 27 owned files against base `5beb2c791a929e5343576dfc3e26d7a831e2087f`, using `review-source.patch` and the new ADR/note. All source hashes match `review-scope.json`; none of these files overlaps the 246 protected WIP files.

The implementation follows ADR 0011's separation of real addresses from socket identity: `vendor/noq-proto/src/lib.rs:438` adds private socket metadata, while `vendor/iroh/src/socket/transports/ip/pool.rs:193` retains actual receive addresses. It keeps the existing descriptor owner, rejects unsupported explicit bindings (`vendor/noq/src/runtime/mod.rs:134`), and includes socket identity in pending-transmit permission checks (`vendor/noq/src/connection.rs:1794`). Migration refreshes the binding through existing eligibility rules (`vendor/noq-proto/src/connection/mod.rs:5847`); bound probes reject prior generations (`vendor/noq-proto/src/connection/mod.rs:2892`). These preserve ADR 0011's rules that identity does not confer permission, explicit binding does not silently fall back, and migrated paths cannot reuse old probe proof.

The new ADR records the authorized local decision explicitly, as required by `AGENTS.md` and `docs/agents/domain.md`. The implementation note describes the remaining demo validation without claiming the separate M69/M84 matrix is complete.

Heuristic smell findings: **none warranting a change**. The added wrappers and delegating compatibility methods have concrete roles in preserving existing public shapes and carrying socket metadata; the baseline does not justify an unsolicited redesign.

Read-only review. No Cargo invocation or source/test/manifest edits. Final demo validation remains outside this report.
