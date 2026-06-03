## Why

Current POC v1 CLI parsing exposes path-related flags such as `-u` and `-t`, and existing reachability docs already describe `-4` and `-6`, but the current runtime path establishment does not consistently carry those per-command constraints into peer session creation. This makes real debugging ambiguous: default IPv6 direct paths can hide IPv4 direct or IPv4 punching behavior, and unsupported TCP-only requests can be silently misleading.

This change makes per-command P2P path overrides explicit, observable, and enforced by the POC v1 runtime without changing signaling or restoring TCP punching.

## What Changes

- Add CLI support for `-4`, `-6`, and `--p2p-ip-family auto|v4|v6` on current POC v1 peer commands that establish or reuse peer sessions.
- Ensure existing `-u`, `-t`, and `--p2p-network auto|udp_only|tcp_only` options are passed from CLI/LocalAPI action arguments into runtime peer session establishment.
- Enforce that P2P path overrides constrain only P2P candidate gathering, direct reachability, punching, and session reuse; they do not constrain MQTT, enrollment, invitation, or control-plane signaling connectivity.
- Keep POC v1 UDP-only: `udp_only` remains supported, while explicit `tcp_only` returns a clear unsupported-path error instead of silently falling back or being ignored.
- Prevent explicit path-constrained commands from reusing an existing peer session whose selected path does not satisfy the requested P2P network or IP family.
- Update documentation and diagnostics so debug runs can prove whether a command selected IPv4, IPv6, direct, or punching behavior.
- Expose the same per-command path policy in the desktop GUI and Android Control Lite demo surfaces without changing non-P2P control actions.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-v1-headless-runtime`: Current v1 action arguments and peer session reuse must honor per-command P2P network and IP-family overrides.
- `miopunch-poc-v1-dial-punch`: Current v1 UDP direct/punch establishment must consume per-command P2P policy when gathering candidates and attempting paths.
- `miopunch-public-reachability`: Existing `-4` and `-6` public reachability semantics must apply to current POC v1 peer commands without affecting signaling connectivity.

## Impact

- Affected CLI surfaces: `ping`, `sh ls`, non-interactive `sh`, and interactive `sh` peer selection arguments.
- Affected runtime/local API arguments: `PingArgs` and `ShellArgs`.
- Affected path-establishment plumbing: POC v1 runtime peer session cache, punch config construction, UDP candidate gather, and UDP attempt policy.
- Affected docs/tests: CLI help, public-network runbook, OpenSpec deltas, parser tests, runtime policy tests, desktop frontend tests, Android demo docs, and clean two-node smoke validation.
