## Why

MNT-01 shows UDP NAT2/NAT3 punching can fail after MQTT exchange because the current executor behaves like a one-shot send/wait script. The product needs a backend-neutral punching phase scheduler so role timing, receive-first behavior, retries, and diagnostics are explicit and reusable by UDP and TCP.

## What Changes

- Introduce a receive-first, bounded punching phase scheduler model for UDP and TCP attempts.
- Move role-aware timing out of MQTT signaling and into decision/executor-owned phase plans.
- Add bounded probe loops, cancellation on winner, and diagnostics for receive/probe lifecycle.
- Add daemon-lifetime success-only analyzer memory for MQTT/task paths.
- Fix the MNT-01 IPv6-to-UDP4 fallback case expectation alongside this scheduler work.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-punching-decision`: phase plans and analyzer memory drive role/delay/budget behavior.
- `miopunch-mqtt-signaling`: MQTT remains exchange readiness only and consumes decision-derived phase plans.

## Impact

- Affected code:
  - `internal/punchdecision`, `internal/punching`, `connectivity`, MQTT task signaling glue, MNT-01 cases.
- Affected tests:
  - UDP NAT2/NAT3 MNT-01 cases.
  - Phase scheduler unit tests and diagnostic event expectations.
- Validation:
  - Full host gates and MNT-01 smoke/selftest after implementation.
