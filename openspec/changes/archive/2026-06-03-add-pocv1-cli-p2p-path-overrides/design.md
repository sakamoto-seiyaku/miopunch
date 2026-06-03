## Context

Current POC v1 path establishment is driven through CLI verbs backed by the shared daemon and LocalAPI runtime. The existing command surface already has partial P2P network parsing (`-u`, `-t`, `--p2p-network`), and project specs/docs already describe IP-family overrides (`-4`, `-6`) as P2P-only constraints.

The gap is that current POC v1 peer actions do not consistently carry these per-command constraints into session establishment. In the current debug environment, default same-host execution can converge through `direct_ipv6`; without an enforced `-4` policy, IPv6 direct can mask whether IPv4 direct or IPv4 punching works. Likewise, accepting `tcp_only` without an implemented TCP path creates misleading behavior.

## Goals / Non-Goals

**Goals:**

- Make `ping`, `sh ls`, and `sh` carry per-command P2P network and IP-family policy from CLI/LocalAPI action args into runtime session establishment.
- Keep the default behavior unchanged when no explicit path override is supplied.
- Ensure explicit `-4` and `-6` constrain candidate gathering, direct reachability, punching attempts, selected path compatibility, and session reuse.
- Make `tcp_only` fail explicitly in current POC v1 because this change does not restore TCP punching.
- Preserve the architectural boundary: signaling/control-plane connectivity remains independent of P2P path overrides.

**Non-Goals:**

- Do not implement TCP direct or TCP punching.
- Do not introduce relay behavior or change MQTT broker connectivity.
- Do not change enrollment, invitation, approval, roster, or credential semantics.
- Do not make `-4` guarantee `punching_ipv4`; it guarantees IPv6 P2P paths are excluded.

## Decisions

1. Per-command path policy is part of runtime action arguments.

   `PingArgs` and `ShellArgs` will carry both `p2p_network` and `p2p_ip_family`. This keeps CLI, LocalAPI, and future GUI entrypoints aligned on the same runtime contract instead of making CLI-only behavior.

2. IP-family parsing reuses existing connectivity semantics.

   The implementation will use the existing `connectivity.P2PIPFamily` and parser semantics: empty/`auto`, `4`/`v4`/`ipv4`, and `6`/`v6`/`ipv6`. This avoids introducing another enum or a second interpretation of `-4/-6`.

3. Explicit path constraints must participate in session cache compatibility.

   If no override is supplied, a healthy existing session may be reused. If an override is supplied, the runtime must verify that the existing session satisfies the requested P2P network and IP family. If it does not, the runtime must establish a fresh session under the requested policy rather than silently reusing the incompatible path.

4. `tcp_only` is unsupported, not silently ignored.

   Current POC v1 is UDP-only. An explicit `tcp_only` request will return an actionable unsupported-path error. This is stricter than falling back to UDP because the command requested a diagnostic constraint.

5. P2P overrides do not affect signaling.

   `-4`, `-6`, `-u`, and `-t` apply only after peer discovery and control-plane message delivery are available. MQTT, STUN resolution, enrollment, invite, approve, and roster lookup keep their existing connectivity behavior.

6. Demo surfaces are thin clients over the same action contract.

   The desktop GUI forwards selected `p2p_network` and `p2p_ip_family` values in
   `ping`, `sh ls`, and `sh` LocalAPI action args. Android Control Lite maps the
   same selections to CLI `-u|-t` and `-4|-6`. Neither surface may apply these
   selections to runtime startup, join/invite, roster `ls`, MQTT, or STUN setup.

## Risks / Trade-offs

- Explicit `-4` may still select `direct_ipv4` instead of `punching_ipv4` on same-host or same-LAN runs -> document this clearly and use selected-path evidence to distinguish the result.
- Rebuilding an incompatible cached session can close a healthy session sooner than the default flow would -> apply this only when the user supplied an explicit diagnostic override.
- Rejecting `tcp_only` may expose previously hidden assumptions in tests or scripts -> update tests to assert the explicit unsupported error instead of relying on silent fallback.
- The feature spans CLI parsing, runtime args, session caching, and punching config -> cover each boundary with focused tests before relying on smoke output.
