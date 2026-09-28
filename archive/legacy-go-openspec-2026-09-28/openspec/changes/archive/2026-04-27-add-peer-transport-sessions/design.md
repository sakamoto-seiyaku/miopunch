## Context

`dataplane.DialStream` and `ServeStream` currently return `io.ReadWriteCloser`. The caller treats that value as both transport carrier and operation stream. For `ping`, acceptor writes the response and returns, then deferred close can close KCP and UDP before the caller observes the response.

FRP and gonc both point to a more stable model: upgrade the punched path into a session, then open per-operation streams over that session.

## Goals / Non-Goals

**Goals:**

- Separate carrier/session lifetime from operation stream lifetime.
- Make KCP, TCP, and QUIC follow one session/logical-stream model.
- Keep logical streams generic enough for future service kinds.
- Preserve existing shell operations during transition.

**Non-Goals:**

- No proactive FRP `keepTunnelOpenWorker` in this round.
- No persistent endpoint/session metadata across daemon restarts.
- No socks/http/https front-proxy punching implementation.
- No QUIC-in-TLS wrapping.

## Decisions

### 1. On-demand live sessions

The daemon owns live peer sessions in memory. Operations open logical streams on an existing healthy session; if no healthy session exists, the operation creates a fresh punching/session round.

### 2. Transport mapping

- TCP: carrier connection, then TLS 1.3 identity binding, then yamux.
- KCP: UDP path, then KCP carrier, then TLS 1.3 identity binding, then yamux.
- QUIC: QUIC native TLS 1.3 identity binding and native streams.

### 3. Stream-open authorization is generic

Each logical stream starts with `kind + metadata`. Authorization happens at stream open. `shellproto` is payload for `shell.v0`, not a mandatory transport prelude.

### 4. Close semantics are explicit

Closing a logical stream ends one operation. Session manager closes the peer session for idle timeout, auth/config changes, daemon shutdown, or transport fatal errors.

## Risks / Trade-offs

- [Risk] Session cache can create concurrency complexity. -> Keep in-memory only, keyed per peer/protocol/path, and close on uncertainty.
- [Risk] yamux close semantics need validation. -> Add focused tests around stream close versus session close.
- [Risk] Existing shell hello migration can be large. -> Allow transitional shell hello payload while introducing generic stream-open auth.
