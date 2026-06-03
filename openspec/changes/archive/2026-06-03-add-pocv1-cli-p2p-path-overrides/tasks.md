## 1. CLI and Action Argument Plumbing

- [x] 1.1 Add `P2PIPFamily` to current v1 `PingArgs` and `ShellArgs`, using the existing `connectivity.P2PIPFamily` string values on the wire.
- [x] 1.2 Extend `ping` argument parsing to accept `-4`, `-6`, and `--p2p-ip-family auto|v4|v6`.
- [x] 1.3 Extend `sh ls` argument parsing to accept `-4`, `-6`, and `--p2p-ip-family auto|v4|v6`.
- [x] 1.4 Extend interactive `sh` peer argument parsing to accept `-4`, `-6`, and `--p2p-ip-family auto|v4|v6`.
- [x] 1.5 Reject conflicting explicit flags such as `-4 -6` and `-u -t` before invoking runtime actions.
- [x] 1.6 Update CLI usage/help text so `ping`, `sh ls`, and `sh` document `[-u|-t] [-4|-6]` and the long options.

## 2. Runtime Policy Enforcement

- [x] 2.1 Normalize per-command path policy in the current v1 runtime with defaults `p2p_network=auto` and `p2p_ip_family=auto`.
- [x] 2.2 Pass normalized path policy from `doPing`, `doShellList`, and `doShell` into peer session establishment.
- [x] 2.3 Reject explicit `tcp_only` with an actionable unsupported-path failure before UDP path establishment.
- [x] 2.4 Make explicit path policy part of peer session cache compatibility checks.
- [x] 2.5 Establish a fresh peer session when an existing session does not satisfy explicit `p2p_network` or `p2p_ip_family` policy.

## 3. Punch Configuration and Path Selection

- [x] 3.1 Add per-command `p2p_network` and `p2p_ip_family` fields to the current v1 punch configuration path.
- [x] 3.2 Ensure UDP snapshot gathering honors requested `p2p_ip_family` instead of deriving family only from available sockets.
- [x] 3.3 Ensure UDP direct and punching attempts honor requested `p2p_ip_family`.
- [x] 3.4 Ensure `p2p_ip_family=v4` cannot select `direct_ipv6`.
- [x] 3.5 Ensure `p2p_ip_family=v6` cannot select `direct_ipv4` or `punching_ipv4`.
- [x] 3.6 Include effective `p2p_network`, effective `p2p_ip_family`, and selected path in debug evidence/logging for peer session establishment.

## 4. Tests

- [x] 4.1 Add parser tests for `ping -4`, `ping -6`, `--p2p-ip-family`, `-4 -6`, `-u`, `-t`, and `-u -t`.
- [x] 4.2 Add parser tests for `sh ls` and interactive `sh` with the same path override cases.
- [x] 4.3 Add runtime tests proving `PingArgs` and `ShellArgs` path policy reaches punch configuration.
- [x] 4.4 Add runtime/session tests proving explicit IPv4 policy does not reuse an existing IPv6 peer session.
- [x] 4.5 Add dial/punch tests proving IPv4-only policy excludes IPv6 candidates and IPv6-only policy excludes IPv4 candidates.
- [x] 4.6 Add tests proving explicit `tcp_only` returns unsupported instead of silently falling back to UDP.

## 5. Documentation and Validation

- [x] 5.1 Update user-facing docs/runbooks to state that `-4`, `-6`, `-u`, and `-t` constrain only P2P path establishment, not signaling.
- [x] 5.2 Update docs to state that POC v1 currently rejects explicit `tcp_only` because TCP punching is out of scope.
- [x] 5.3 Run `openspec validate add-pocv1-cli-p2p-path-overrides --strict`.
- [x] 5.4 Run focused Go tests for CLI parsing, current v1 runtime, punch, session, and connectivity packages.
- [x] 5.5 Rebuild a clean bundle, start two clean debug-level daemon instances, and verify default `ping`, `ping -6`, and `ping -4` behavior with selected-path evidence.

## 6. Demo Surface Follow-through

- [x] 6.1 Add desktop GUI controls for `p2p_network=auto|udp_only|tcp_only` and `p2p_ip_family=auto|v4|v6` on P2P peer action surfaces.
- [x] 6.2 Ensure desktop GUI `ping`, `sh ls`, and `sh` LocalAPI actions include the selected path policy.
- [x] 6.3 Add Android Control Lite selectors for the same path policy and map them to CLI `-u|-t` and `-4|-6`.
- [x] 6.4 Keep join/invite/roster/runtime actions unaffected by demo path selectors.
- [x] 6.5 Add desktop frontend coverage proving selected policy reaches RuntimeAction args.
- [x] 6.6 Update Android demo docs/runbook with the path selector scope and ADB extras.
