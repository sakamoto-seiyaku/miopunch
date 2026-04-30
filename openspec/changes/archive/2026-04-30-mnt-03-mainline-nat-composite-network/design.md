## Context

MNT-01 validates real mainline two-node connectivity over controlled NAT profiles. MNT-02 validates product control-plane e2e behavior with multiple real nodes, but it does not prove complex NAT overlay formation, bootstrap selection, active neighbor maintenance, or perturbation recovery.

`docs/decisions/mainline-network-test-charter.md` defines MNT-03 as the third required scenario: a 12-node NAT composite network using real `miopunch` daemon/product CLI nodes. The lab is only the network fixture. Mainline product behavior must own bootstrap, presence, reachability hints, neighbor selection, payload validation, and recovery.

## Goals / Non-Goals

**Goals:**

- Implement the complete MNT-03 scenario with 12 real Docker/systemd `miopunch` nodes behind controlled NAT/link profiles.
- Add mainline product behavior for `bootstrap_more`, presence/online, reachability hints, logN active neighbor maintenance, active-edge evidence, and recovery reporting.
- Expose stable LocalAPI/CLI topology diagnostics for gates and operators.
- Add `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest` lab gates with artifacts pulled to `lab/_artifacts/`.

**Non-Goals:**

- Do not use lab-only helper state to choose bootstrap candidates, maintain neighbors, or determine success.
- Do not let the lab write product semantic state. Lab writes are limited to audited infrastructure fixture fields needed to attach a real node to the controlled NAT/MQTT/STUN environment.
- Do not require public MQTT brokers.
- Do not expand GUI or shell feature matrices beyond minimal payload evidence needed by MNT-03.
- Do not make Docker bridge networking the NAT model.

## Decisions

1. **Product-only group semantics**

   Mainline `miopunch` owns bootstrap, presence, reachability hints, neighbor selection, active-edge validation, and recovery. Lab code may start nodes, attach networks, inject faults, collect artifacts, and set audited infrastructure fixture fields such as MQTT/STUN/P2P port and IP-family settings. It must not create peers, membership, governance, decls, bootstrap candidates, neighbor state, selected paths, payload success, or recovery conclusions.

2. **Docker/systemd nodes attached to lab NAT**

   Each node is a Docker container running systemd and the real `miopunch` system daemon. The harness attaches the container network namespace to a lab-controlled NAT domain using veth. NAT rules, pcap, conntrack, tc, MQTT, and STUN remain under lab control.

3. **Stable topology diagnostics through LocalAPI**

   Add `GET /api/v0/topology` and CLI JSON output backed by the same product data. The response carries bootstrap, reachability, active neighbor, attempt, payload, and recovery evidence. Gates must consume these interfaces instead of parsing daemon logs or private state files as the primary signal.

4. **Adopt existing product discussion defaults**

   MNT-03 uses the existing defaults from `docs/notes/2026-04-15-alpha-product-discussion.md` where the charter needs concrete behavior: two initial bootstrap candidates, at most two `bootstrap_more` rounds, two candidates per successful round, `bootstrap_more` timeout of 5s, presence with state-head evidence, reachability hint ordering, and `k=max(2,ceil(ln(n)))`.

5. **Incremental milestone execution**

   Implementation proceeds through real lab milestones: 2-node substrate, 3-node bootstrap, 4-node reachability/portmap, 6-node bootstrap_more/hard carry, and then 12-node full topology. Each milestone must run in the QEMU lab VM with real Docker/systemd `miopunch` nodes before adding the next class of complexity.

6. **Layered gates map to milestone groups**

   `mnt03-smoke` covers the 2-node and 3-node milestones. `mnt03-selftest` covers the 4-node and 6-node milestones. `mnt03-fulltest` covers the complete 12-node topology plus perturbation and recovery with pcap/conntrack/tc evidence.

## Risks / Trade-offs

- [Risk] Docker/systemd + veth/NAT wiring is operationally complex. -> Mitigation: build a small reusable topology library first, emit topology manifests, and collect network namespace/container inspect output for every run.
- [Risk] Product background behavior may introduce races in tests. -> Mitigation: expose bounded budgets and state through LocalAPI topology diagnostics, and make gates wait on explicit stabilization conditions.
- [Risk] Full 12-node fulltest may be expensive. -> Mitigation: keep smoke/selftest as lower-cost gates and reserve full perturbation/pcap coverage for `mnt03-fulltest`.
- [Risk] Existing LocalAPI route-set spec was intentionally minimal. -> Mitigation: add one versioned topology diagnostics route instead of ad hoc debug endpoints.

## Migration Plan

1. Update charter and OpenSpec artifacts.
2. Implement product data models and LocalAPI/CLI topology diagnostics.
3. Build the 2-node substrate and run it in the lab VM before adding more nodes.
4. Expand through the 3-node, 4-node, and 6-node milestones, adding only the product behavior needed by each milestone.
5. Expand to the full 12-node topology and add perturbation/recovery coverage.
6. Add host/guest gate commands and run gates from smoke to fulltest.

## Open Questions

- None for the OpenSpec contract. Implementation may discover product defects; those should be recorded in `docs/notes/mainline-network-test-findings.md` unless they are part of the MNT-03 required behavior.
