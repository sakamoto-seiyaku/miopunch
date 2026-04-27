## Context

UDP candidate exchange already separates direct addresses from assisted/private addresses. TCP did not receive the same split during Door 2, so private TCP listen addresses can enter `direct_tcp4` attempts and confuse both product behavior and tests.

F-002's direct/portmap cases are not valid direct coverage until their fixtures produce true direct candidates. F-005 is the product fix that makes this visible and correct.

## Goals / Non-Goals

**Goals:**

- Add TCP assisted candidate semantics.
- Prevent private TCP listen addresses from being attempted as direct.
- Keep assisted fallback bounded and diagnostic.
- Correct MNT-01 case names/expectations around TCP direct versus fallback.

**Non-Goals:**

- No old-node migration.
- No change to TCP `P/P+100` convention.
- No new `assisted_tcp4` path name.
- No broad TCP spraying retuning.

## Decisions

### 1. Source determines direct eligibility

`tcp_direct_addrs` is not “any TCP address we know.” It is a bucket for true direct candidates: public/routeable TCP direct, TCP portmap direct, and eligible IPv6 direct. Private IPv4 listen addresses belong to `tcp_assisted_addrs`.

### 2. Assisted targets are exact targets

Assisted addresses can be dialed as exact punching targets. They are not inputs to candidate port range expansion or random spraying.

### 3. Assisted-only fallback is mode0

When TCP STUN evidence is insufficient but assisted targets exist, decision can enable minimal mode0 best-effort punching. It must not claim NAT analysis success and must explain the fallback.

### 4. Tests must match fixture truth

Existing MNT-01 TCP direct/portmap cases should stop claiming direct coverage unless fixture support exists. Fallback success is useful, but it must be named and asserted as punching/fallback.

## Risks / Trade-offs

- [Risk] New field semantics touch wire/gather/decision/attempt together. -> Keep field names parallel to UDP and reject obvious private addresses in direct output.
- [Risk] Assisted fallback may be misread as direct success. -> Keep path as `punching_tcp4` and emit target-source diagnostics.
