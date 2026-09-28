## Why

MNT-01 has proven mainline two-node connectivity and MNT-02 has proven the control-plane e2e loop, but the mainline still lacks an end-to-end proof that real `miopunch` daemon nodes can form and maintain a multi-NAT network. MNT-03 closes that gap by validating blank startup, network formation, bootstrap, reachability-aware neighbor maintenance, active-edge punching, and recovery in a controlled 12-node NAT universe.

## What Changes

- Add MNT-03 as the required 12-node mainline NAT composite network gate.
- Use real Docker/systemd `miopunch` product nodes as the system under test; lab provides only NAT/MQTT/STUN/pcap/conntrack/netem fixtures and must not inject membership, bootstrap, neighbor, reachability, topology, or success state.
- Wire Docker node network namespaces into lab-controlled NAT domains via veth so Docker does not bypass NAT behavior through a flat bridge.
- Implement missing mainline behavior needed by the charter: `bootstrap_more`, presence/online state, reachability hints, logN active neighbor maintenance, active-edge validation, and recovery evidence.
- Expose stable LocalAPI/CLI JSON observability for bootstrap, reachability, active neighbors, degree distribution, attempt path, data protocol, payload evidence, and recovery outcomes.
- Add layered MNT-03 gates: `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest`.
- Use `docs/decisions/mainline-network-test-charter.md` as the scenario source of truth and `docs/notes/2026-04-15-alpha-product-discussion.md` as the default source for bootstrap/presence/reachability/logN parameters.

## Capabilities

### New Capabilities

- `miopunch-mainline-nat-composite-network-v0`: Defines the MNT-03 12-node NAT composite network, fixture boundary, node profiles, gate layers, required product behavior, evidence, and recovery acceptance criteria.

### Modified Capabilities

- `miopunch-poc-localapi-v0`: Adds stable topology/diagnostic LocalAPI and CLI JSON observability needed by MNT-03.
- `miopunch-poc-control-plane-mailbox`: Adds control-plane delivery requirements for presence and bootstrap_more over the untrusted MQTT mailbox.
- `miopunch-poc-control-plane-rpc-time-semantics`: Adds bounded RPC timeout/retry semantics for bootstrap_more and state/recovery RPCs.
- `miopunch-poc-control-plane-mesh-first-fallback`: Extends mesh-first control-plane behavior to active neighbor maintenance and recovery delivery.
- `miopunch-poc-decls-v0`: Adds reachability hint reporting and state-head evidence needed for peer selection and convergence checks.

## Impact

- Affected product behavior:
  - Daemon control-plane background behavior, bootstrap selection, presence, neighbor maintenance, recovery, and report observability.
- Affected LocalAPI/CLI:
  - New or extended JSON endpoints/commands for topology, bootstrap, reachability, active neighbors, and recovery evidence.
- Affected lab/runtime:
  - New MNT-03 Docker/systemd + NAT lab topology, host `labctl` entries, guest runners, artifacts, and pcap/conntrack/tc collection.
- Affected validation:
  - Adds `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest`.
  - Code-affecting implementation requires normal Go gates plus the new lab gates.
- Out of scope:
  - GUI and shell feature matrices beyond the minimum payload evidence needed for network validation.
  - Public MQTT brokers as required gates.
  - Lab-only bootstrap or neighbor selection substitutes.
