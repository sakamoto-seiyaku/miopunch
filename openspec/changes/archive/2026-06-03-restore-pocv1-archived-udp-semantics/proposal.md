## Why

POC v1 restored UDP decision inputs, but the active implementation still does
not preserve the archived working UDP punching semantics: runtime-owned UDP
sockets, temporary random-listen winners, KCP session handoff, assisted
candidates, UDP6 direct, and analyzer feedback are still mixed or rewritten.

This must be fixed before further CLI/GUI smoke work, because the current path
can report a successful punch while the secure session fails or later retries
operate on the wrong ownership model.

## What Changes

- Restore the archived UDP selected-socket contract: runtime-owned UDP winners
  and temporary random-listen winners are distinct handoff cases.
- Make Runtime the sole owner of the long-lived UDP owner/demux and route
  traversal and KCP packets through one boundary.
- Make POC v1 secure session consume owner-safe KCP transport views instead of
  directly reading Runtime raw `*net.UDPConn`.
- Preserve mode2/mode4 temporary UDP winner ownership through secure-session
  success and failure paths.
- Restore archived assisted/private candidate semantics instead of POC v1-only
  interface-name filtering.
- Restore UDP6 direct support unless an explicit IPv4-only exception is added.
- Fix daemon analyzer success feedback so each peer records success in its local
  remote-peer scope.
- Replace tests that currently protect the wrong no-op close, original-conn-only
  handoff, and virtual-interface filtering assumptions.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-v1-dial-punch`: `PathResult` and UDP attempt handoff must
  distinguish runtime-owned sockets from temporary random-listen winners and
  must preserve archived UDP gather/attempt semantics.
- `miopunch-poc-v1-secure-session`: KCP session upgrade must consume an
  owner-safe packet transport for runtime-owned UDP paths and must own temporary
  UDP winners on success/failure.
- `miopunch-udp-socket-owner-demux`: POC v1 runtime-owned UDP sessions must use
  one owner/demux for traversal and KCP, with no competing raw UDP readers.
- `miopunch-poc-v1-headless-runtime`: Runtime must own the UDP owner lifecycle,
  expose selected-path/session-handoff failures, and preserve actionable
  evidence for punch-to-secure-session failures.
- `miopunch-punching-decision`: daemon analyzer success feedback must be scoped
  to each local peer's remote-peer/protocol view.

## Impact

- Affected implementation areas: `internal/pocv1/punch`,
  `internal/pocv1/session`, `internal/pocv1/runtime`,
  `internal/punching`, `internal/udpowner`, `connectivity`, and
  `internal/punchdecision`.
- Affected contracts: `PathResult` handoff, Runtime UDP owner lifecycle,
  session close/cleanup, UDP6 direct capability, assisted candidate collection,
  and analyzer success reporting.
- No CLI syntax changes.
- No TCP punching or CN-STUN arbitration is reintroduced.
- Requires focused Go validation before real CLI smoke; lab/VM gates are follow-up
  after the owner/handoff contract is proven.
