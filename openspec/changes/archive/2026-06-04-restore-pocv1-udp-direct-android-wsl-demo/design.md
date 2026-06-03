## Context

The archived POC dial path gathered direct UDP/TCP candidates, exchanged a `NatHoleResp`, and used `connectivity.Attempt` to try direct paths before UDP punching. That direct-first behavior is why a same-LAN pair could succeed without needing NAT detect convergence.

Current POC v1 deliberately narrowed the model: `dial_offer` / `dial_answer` exchange only v1 candidates, then `internal/pocv1/punch` runs bounded UDP candidate-pair attempts through `punching.MakeHole`. The only current direct shortcut is `mirroredHostRemoteAddr`, which covers the WSL/Windows mirrored case where both candidates have the same IP with different ports. It does not cover Android/WSL same-LAN candidates such as `192.168.4.151:43615` and `192.168.4.5:48438`.

Real Android/WSL evidence shows raw UDP and TCP are reachable both ways, but `miopunch ping` fails at `Punch` with `wait detect message error: context deadline exceeded`. The first restoration step should therefore be UDP direct reachability, not TCP Door-2.

## Goals / Non-Goals

**Goals:**

- Restore Android/WSL demo viability by trying UDP direct IPv4 before UDP punching for current POC v1 host candidates.
- Keep the v1 `dial_offer` / `dial_answer` wire body unchanged.
- Preserve one UDP socket owner / demux boundary for traversal and later session handoff.
- Make selected path evidence visible in CLI/report output and trace logs.
- Keep focused tests and real-device validation scoped to the Android/WSL path.

**Non-Goals:**

- Do not restore TCP direct, TCP punching, `p2p_network` policy enforcement, STUN mapped candidates, port mapping, relay, QUIC carrier selection, or the full archived `connectivity.Gather` / `connectivity.Attempt` stack.
- Do not add Android UI, HTTP panel work, or app packaging in this change.
- Do not change enrollment, roster, presence authority, or the `dial_offer` / `dial_answer` identity validation model.

## Decisions

1. **Implement direct UDP inside `internal/pocv1/punch`, not by importing the full archived attempt stack.**

   Current POC v1 owns a narrower product path and already has a pair-plan abstraction, shared UDP socket, and traversal demux. Reusing only the direct handshake pattern avoids pulling TCP/STUN/policy semantics back into v1 before they are intentionally designed.

   Alternative considered: call `connectivity.Attempt` directly. That would restore more behavior quickly, but it would also reintroduce TCP policy, STUN fields, direct/mapped/assisted candidate semantics, and dataplane recipe assumptions that current POC v1 explicitly excluded.

2. **Run direct UDP before `punching.MakeHole` for IPv4 host-host candidate pairs.**

   The Android/WSL failure is a same-LAN host-candidate case. The direct attempt should use the same UDP socket and the same encrypted/tagged traversal message format, then fall back to existing UDP punching on timeout or non-host candidates.

   Alternative considered: extend `mirroredHostRemoteAddr` to cover same subnet. That would be a false positive because same subnet does not prove the remote UDP socket can receive and respond. A real direct handshake is safer and gives better diagnostics.

3. **Expose path evidence as first-class punch evidence.**

   Operators need to know whether a demo succeeded by direct UDP or punching. The selected path should appear in success facts/report data, and each attempted pair should record the path and result.

4. **Add trace diagnostics at the traversal boundary.**

   `--log-level trace` must be sufficient to answer whether a packet was sent, received, decoded, routed to an endpoint, dropped due to unknown transaction, or selected as the winner. This is more useful than only logging final timeouts.

## Risks / Trade-offs

- **Direct UDP could mask punching regressions** -> Tests must cover direct success and direct-timeout fallback to punching.
- **More trace logs could be noisy** -> Keep new packet-routing logs at trace level and avoid logging payload bytes or credentials.
- **Current v1 still lacks TCP policy despite `-t` parsing** -> Document this explicitly and keep `ping -t` outside this change's acceptance criteria.
- **Android root/ADB setup remains required for the current demo** -> Treat that as real-environment validation setup, not as product UI scope.

## Migration Plan

Implement as a normal current POC v1 behavior change. Existing UDP punching remains as fallback, so rollback is to disable/remove the direct attempt and return to punching-only behavior.

No persisted state migration, broker migration, or wire migration is required.

## Open Questions

- None for this change. TCP Door-2 restoration and Android app packaging should be separate follow-up changes.
