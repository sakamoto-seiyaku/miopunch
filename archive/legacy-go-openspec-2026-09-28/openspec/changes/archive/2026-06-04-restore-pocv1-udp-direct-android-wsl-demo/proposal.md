## Why

Android arm64 CLI has been proven to execute, join the network, approve, discover the peer, and exchange raw UDP/TCP packets with WSL. The current failure is narrower: POC v1 attempts UDP punching directly for a same-LAN Android/WSL host-candidate pair and times out, while the archived POC stack first tried UDP direct reachability before falling back to punching.

This change restores the smallest missing path needed for a credible Android/WSL demo without reintroducing the full archived TCP Door-2 stack.

## What Changes

- Add a current POC v1 UDP direct attempt before UDP punching for IPv4 host-to-host candidate pairs.
- Keep current POC v1 carrier scope UDP-only; do not add TCP direct, TCP punching, relay, QUIC carrier selection, or STUN mapped/assisted candidate gathering in this change.
- Preserve the existing `dial_offer` / `dial_answer` exchange shape while using the exchanged host candidates for direct UDP reachability first.
- Add path evidence so CLI/report output can distinguish `direct_ipv4` from `punching_ipv4`.
- Improve trace diagnostics around direct UDP attempts and UDP traversal demux/punching so Android/WSL failures are stage-locatable.
- Update POC v1 documentation/specs to describe the restored direct-first behavior and the Android/WSL validation target.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-v1-dial-punch`: current v1 UDP path establishment changes from punching-only to UDP direct-first with UDP punching fallback.
- `miopunch-poc-v1-headless-runtime`: runtime/CLI evidence must expose the selected UDP path for demo/debug output.
- `miopunch-udp-socket-owner-demux`: traversal demux observability must make direct/punch packet routing decisions diagnosable at trace level.

## Impact

- Affected implementation areas: `internal/pocv1/punch`, `internal/pocv1/runtime`, `internal/udpowner`, and CLI/report evidence formatting in `cmd/miopunch`.
- Affected validation: focused Go tests for POC v1 punch/runtime/CLI plus a real Android/WSL run using trace logs.
- No breaking CLI syntax changes.
- No new external dependencies.
