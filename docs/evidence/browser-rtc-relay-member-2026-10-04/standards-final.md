# S5 Standards recheck

2026-10-04, read-only narrow followup. HEAD remains
`f5bae5c727c632faf5cac0b3473b01cbbe938fe4`; the review target includes tracked
and untracked WIP against that fixed point.

## Required findings

None. No new documented hard standards breach identified. The current
AGENTS/domain guidance and ADR0004/0005 remain the standards baseline; archived
Go/OpenSpec gates and tooling-enforced formatting/type checks remain excluded.
The implementation remains explicitly experimental, without a formal/default
RTC backend or product-deadline decision.

The shared `RtcPumpGuard` retires exact IO on driver exit and accounts for route
and transport task counts. `while_active` centralizes resource retirement,
original-M closure and qualification-notification cancellation; native/browser
sends recheck current IO. Native `PathQualification` retains independent atomic
revision reads for packet callbacks. Browser/native notification updates precede
authorization-policy mutation. The custom-carrier opt-in preserves existing
carriers' ordinary-frame behavior.

## Optional recommendations

**Possible Mysterious Name** — `src/rtc_member.rs:537` writes:

```rust
"ownedPumpTasks": transport.active_pumps()
```

This is a global transport count, whereas `lease.report().owned_pump_tasks`
counts this route's drivers. If another M still has a pump, this cleanup receipt
can suggest the completed attempt still owns tasks. Consider separate fields:
`ownedPumpTasks` from the lease and `activePumpTasks` from the transport, or
rename the global field. This is a diagnostic naming judgement, not an
acceptance gate.

The initial duplicated-statistics and callback-detachment recommendations are
parked; no additional refactor recommendation is introduced.

## Readback evidence

Read the updated guard/retirement implementation and both pump call sites,
native coordinator oneshot observation/cleanup, qualification-notification
changes in both gates, CustomSender/default/resource wrapper and allocation-free
mapped predicates, plus the three new held-datagram tests. The tests cover
resource retirement simultaneous with capacity, scope retirement with capacity
still blocked, and original-M closure on authenticated loopback QUIC.

Inspected fixed-point diff/stat and verified HEAD via RTK-prefixed commands.
No source/index/commit mutation, build, test, Plane access or network traffic
performed. Parent-reported runtime results were not independently rerun.

Result: 0 required findings; 1 new optional naming recommendation.
