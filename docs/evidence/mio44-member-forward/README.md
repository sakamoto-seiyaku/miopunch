# MIOPUNCH-44 evidence

Acceptance exercises real three-node persistent admission and TLS/QUIC through
`tests/member_forward.rs` and the formal native endpoint. The implementation
and declared capability bounds are recorded in
[`2026-10-02-member-signaling-validation.md`](../../notes/2026-10-02-member-signaling-validation.md).

Re-run the ordered checks into a new directory:

```sh
rtk proxy python -B scripts/check_member_forward.py --output /tmp/mio44-new-run --short
```

The runner first checks B's signaling-only assistance, then rejects A-C business
when no other path is permitted. Each later check changes one fault or behavior.
`results.json` records exact commands, exit status, duration and raw logs. The
short child receipt records its owned broker cleanup. No slow unchanged NAT
matrix is repeated.

## Initial RED/GREEN and diagnosed failures

- `sandbox-bind-failure.log`: Linux sandbox disallowed socket creation. This is
  an environment failure, not behavioral RED.
- `first-red.log`: compiled and ran with real sockets. Old admission rejected
  the member signaling transport before ordinary connection delivery, exit101.
- `first-green.log`: shared existing admission/path coordinator permits the
  restricted adapter and forms an actual A-C UDP data connection, exit0.
- `priority-green.log`: control is reachable, but no mutually permitted A-C
  data capability exists; both ordinary connection results fail and A-B/B-C
  business still works, exit0. The pre-existing restriction already passed.
- `lifecycle-and-security.log`: initial8-test extension had5 passes/3 failures.
  Wrong identity reused cached legitimate B, so a separate valid route could
  prevent C's accept branch from receiving a connection. Expiry/interruption
  checks also waited beyond the4-second outer idle timeout without keepalive.
  Neither failure demonstrates successful impersonation or automatic relay.
- `lifecycle-and-security-fixed.log`: an uncached unrelated TLS target and
  explicit outer keepalive isolate the intended faults; all8 tests pass.

## Ordered final check

`final/results.json` and `final/mqtt-short/results.json` record the current
ordered acceptance and affected short regressions. The extra endpoint-close
case checks prompt release of adapter tasks and B's registration while C-B
continues to work. Only an actually completed receipt with `passed:true` counts
as acceptance; partial runs retain `passed:false`.

Both final receipts are `passed:true`: all9 ordered member checks and15 short
groups exit0. The short groups report137 passing tests (including their inert
subprocess worker entries), plus workspace/all-target and browser WASM compile
checks and root/fork formatting. WASM is compile-only. The extra all-target
check includes the new member fault feature; the default member test target
also compiles. Both owned short brokers were removed with exit0. No runtime
source changed after these final checks.

Packet-marker observations are limited evidence, not a security audit. Identity
binding uses the real inner TLS implementation and the ordinary member proof.
The fork's existing restricted data-release and route retirement enforce that
normal business cannot use either message adapter as a fallback. No arbitrary
NAT, multi-hop, browser-runtime or member data-relay success is claimed here.
