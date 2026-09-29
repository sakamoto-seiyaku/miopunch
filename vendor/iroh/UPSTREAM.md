# iroh fork for UDP address observation

This directory starts from the published `iroh` crate version `1.2.0` in the
local crates.io registry. `Cargo.toml` is the published, normalized manifest;
`Cargo.toml.orig` preserves the upstream workspace manifest. The upstream
license files remain in this directory.

MIOPUNCH-19 changes only `src/endpoint.rs` and `src/socket.rs`:

- `Builder::observation_servers` configures QAD servers independently of data
  relay transports. Explicit observation servers run QAD without HTTPS or
  captive-portal probes and cannot be selected as a data relay. This validation
  path requires data relay transports disabled; a mixed configuration fails at
  bind rather than silently altering relay selection.
- Empty observation configuration still permits the independent portmapper.
- `Endpoint::udp_observation` exposes observed UDP candidates, a 40-second
  validity deadline, destination-dependent mapping information, and bounded
  failure states. A fresh state is published after `Endpoint::addr` is updated.
- Periodic full QAD reports replace stale candidates; an explicit
  `Endpoint::network_change` refreshes even if the interface list is unchanged.
  Generation checks discard results from an older in-flight QAD run.

The fork is selected for every workspace dependency with the root
`[patch.crates-io]` entry. See
`docs/notes/2026-09-29-udp-observation-validation.md` for the real UDP and
kernel-NAT evidence and the scope of this validation.
