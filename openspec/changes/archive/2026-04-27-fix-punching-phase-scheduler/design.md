## Context

Current MQTT exchange aligns both peers on a common `start_at`, but that is not enough for NAT2/NAT3. In the observed failure, the sender's normal packet reaches the restricted NAT before the receiver's low-TTL opener has created filtering state; there is no later normal packet to overlap the opened state.

FRP's coord path had implicit role-aware response/start timing. miopunch must not recreate that timing inside MQTT; it must model punching phases explicitly.

## Goals / Non-Goals

**Goals:**

- Make UDP punching receive-first and bounded.
- Preserve TCP's existing receive-first direction and align it with the shared model.
- Keep signaling backend-neutral for future MQTT/NATS/DHT/Git/Email exchange backends.
- Add success-only analyzer memory to MQTT/task paths.

**Non-Goals:**

- No filtering-aware NAT classifier redesign.
- No backend-specific sleep or MQTT-only fix.
- No endpoint/candidate persistence across daemon restarts.

## Decisions

### 1. Phase plan is the executor contract

The decision layer should derive a phase plan containing role, send delay, TTL behavior, targets, budget, probe interval/scale, and diagnostic labels. Executors create receive loops before probe loops.

### 2. UDP and TCP share model, not packet functions

UDP sends SID datagrams and detects SID responses; TCP listens/dials and converges to a connection. Both follow the same phase ordering and cancellation semantics.

### 3. Success-only analyzer memory is conservative

The daemon remembers only successful `mode/index` per peer/protocol/analysis key. It does not remember failures, endpoints, targets, or path source. Every round still gathers and exchanges fresh snapshots.

### 4. F-004 is a test expectation issue

`mnt01-self-ipv6-udp4-fallback` should not expect `direct_ipv4` unless the fixture creates a real IPv4 portmap/direct candidate. That test cleanup is bundled here because it is part of attempt path semantics.

## Risks / Trade-offs

- [Risk] Shared scheduler abstraction can become too broad. -> Keep protocol-specific probe adapters small and local.
- [Risk] More repeated probes may affect runtime. -> Keep budget bounded and observable.
- [Risk] Analyzer memory could mask network changes. -> Store only success mode/index with TTL; never store endpoints or failures.
