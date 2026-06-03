## Why

POC v1 now needs the GUI/headless demo loop to be repeatable: create/join a
network, approve the peer, establish a session, then run ping and shell actions
in either direction without restarting daemons.

The current runtime binds one UDP socket for the daemon, but the punch/session
handoff still treats that socket as if a single `PathResult` owns it. When a
secure-session attempt fails, a peer session is closed, or reverse/concurrent
actions overlap, that borrowed socket can be closed by the wrong layer. The next
punch/session attempt then reuses the same Go pointer but the file descriptor is
already closed, surfacing as `use of closed network connection`.

## What Changes

- Make Runtime the sole owner of the long-lived UDP socket used by POC v1.
- Treat `PathResult` as a borrowed-path descriptor; closing a `PathResult` must
  not close Runtime's UDP socket.
- Treat secure sessions as borrowers of the punched UDP socket; closing a
  `PeerSession` must not close Runtime's UDP socket.
- Reuse a healthy peer session for repeated ping/shell actions and remove a
  transport-fatal session before the next dial/punch.
- Keep the default POC broker as `tcp://broker.emqx.io:1883` when no broker is
  configured.
- Record focused tests for failure and close paths that previously closed the
  shared UDP socket.

## Capabilities

### Modified Capabilities

- `miopunch-udp-socket-owner-demux`: clarifies socket ownership and borrowed
  handoff semantics.
- `miopunch-poc-v1-dial-punch`: clarifies that `PathResult` is not a socket
  owner.
- `miopunch-poc-v1-secure-session`: clarifies that session transport close does
  not close the runtime UDP owner.
- `miopunch-poc-v1-headless-runtime`: requires stale/fatal sessions to be
  removed so subsequent actions can re-punch.

## Impact

- Affected implementation areas: `internal/pocv1/punch`,
  `internal/pocv1/session`, `internal/pocv1/runtime`, and focused notes/tests.
- Affected validation: focused Go tests for punch/session/runtime lifecycle.
- No CLI syntax changes.
- No broker behavior changes beyond preserving the public default.
