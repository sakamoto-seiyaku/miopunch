## Why

Desktop peer details can show an active `tls / tcp4` session while the related reachability facts remain `unknown`. The connection is real, but the product-facing LocalAPI state drops selected endpoint and path evidence before the GUI can render it.

## What Changes

- Carry non-secret selected session path facts from dataplane/session state into `GET /api/v0/desktop/state` and `GET /api/v0/topology`.
- Populate active peer session and active neighbor objects with local/remote endpoints, selected path status, and selected port when evidence is available.
- Preserve conservative unknown behavior for direct IPv4/IPv6 and public tuple fields unless the daemon has explicit validated evidence.
- Expose selected STUN/TCP view evidence in topology attempts when it is already known by the signaling/decision path.
- Rename the GUI's reported `IPv4`/`IPv6` metadata labels so users understand they are reachability hints, not observed IP addresses.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-poc-localapi-v0`: desktop and topology snapshots must expose available selected session path facts instead of dropping them.
- `miopunch-dataplane`: peer session diagnostics must retain safe local/remote endpoint facts for product-facing runtime state.
- `miopunch-desktop-gui-v0`: peer details must label configured reachability hints distinctly from observed path facts.

## Impact

- Affected Go packages: `dataplane`, `internal/task`, `internal/localapi`.
- Affected desktop asset: `cmd/miopunch-desktop/frontend/dist/assets/app.js`.
- No wire protocol or secret material changes.
- Requires Go unit tests plus focused LocalAPI/frontend verification; full gate set is required before mainline commit.
