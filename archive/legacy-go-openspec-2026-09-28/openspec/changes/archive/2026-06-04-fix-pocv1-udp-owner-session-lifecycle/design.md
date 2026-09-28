## Context

The original POC v1 direction was sound: bind one UDP port per daemon, advertise
that port in local candidates, punch a path, and keep using the same local UDP
port for the session. The bug is in ownership precision. The implementation
passed the Runtime-owned `*net.UDPConn` through `PathResult.Conn`, then allowed
`PathResult.Close()` and session transport closers to close it.

That makes normal retry behavior unsafe. A failed TLS/KCP upgrade, peer-session
close, or reverse command can close the daemon UDP socket while Runtime still
stores the old pointer and continues to advertise the old port.

## Decisions

### Runtime owns the UDP socket

Runtime is the only layer that may close the daemon UDP socket. It closes the
socket when the runtime is closed or when a future explicit socket-owner restart
path replaces it.

### PathResult is a borrowed descriptor

`PathResult` carries the selected remote UDP endpoint, trusted remote identity,
and evidence. It may reference the Runtime UDP socket for compatibility with
the current KCP upgrade path, but it does not own that socket. `PathResult.Close`
is retained as a compatibility no-op.

### PeerSession borrows the UDP socket

The session layer may close yamux, TLS, KCP sessions, and listeners it creates.
It must not close the Runtime UDP socket supplied by `PathResult`.

### Fatal sessions are not reused

Runtime reuses a healthy `PeerSession`. If stream open/control I/O reports a
transport-level unavailable failure, Runtime removes the session with
`transport_fatal`; the next action can establish a fresh punched path.

## Non-Goals

- Do not introduce a local MQTT broker for the POC demo; the default remains the
  public EMQX broker unless explicitly configured otherwise.
- Do not redesign KCP conversation demultiplexing in this change. The immediate
  fix is the socket ownership bug that causes the closed UDP file descriptor to
  be reused.
