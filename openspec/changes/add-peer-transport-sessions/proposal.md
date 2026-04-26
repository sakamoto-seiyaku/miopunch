## Why

MNT-01 KCP transport can establish a punching path and complete hello, then time out on ping because the dataplane exposes a bare stream and closes the underlying KCP/UDP carrier at operation end. The dataplane needs peer transport sessions with per-operation logical streams.

## What Changes

- Introduce on-demand live peer transport sessions owned by the daemon/task runtime.
- Use logical streams for individual operations; stream close does not close the peer session.
- Use `TLS 1.3 + smux` for TCP and KCP sessions; use native QUIC streams for QUIC.
- Add generic stream-open `kind + metadata` authorization; keep shellproto as a payload protocol, not the transport/session protocol.
- Tighten KCP transport MNT-01 specialty from diagnostic allowed failure to required `ping=ok`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-dataplane`: peer transport session lifecycle, generic logical streams, stream-open authorization, and KCP/TCP/QUIC session mapping.

## Impact

- Affected code:
  - `dataplane`, `internal/task`, `internal/pocacceptor`, shell protocol integration, KCP/TCP/QUIC transport setup.
- New dependency:
  - `smux` for TCP/KCP multiplexing if not already present in the module.
- Validation:
  - KCP transport MNT-01 case must prove `hello=ok` and `ping=ok`.
