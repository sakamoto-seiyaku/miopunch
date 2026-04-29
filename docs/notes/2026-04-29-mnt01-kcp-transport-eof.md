# 2026-04-29 MNT-01: KCP Transport EOF (CapabilityHandshake)

## Summary

`./lab/host/labctl mnt01-smoke` currently reaches `pass=7 fail=1`.

The only failing case is `mnt01-smoke-kcp-transport`, and it fails during **CapabilityHandshake** with:

- `read stream-open hello response: EOF`

This means the UDP traversal path is selected (e.g. `attempt_path=punching_ipv4`), but the peer transport session built as **KCP + TLS 1.3 (pinned identity) + yamux** closes before the dialer can read the `hello` response on the opened logical stream.

At the moment we cannot see the server-side reason for the close in artifacts (daemon logs are effectively empty), so investigation is stuck in a loop of “dial side sees EOF, but accept side provides no error context”.

## Evidence (Artifacts)

1. Smoke failure (KCP transport)

- Artifact dir: `lab/_artifacts/20260429T062839Z-mnt01-mnt01-smoke-kcp-transport/`
- Key file: `attempt-1.md`
  - `CapabilityHandshake: hello handshake`
  - `read stream-open hello response: EOF`

2. Debug run with LAN/WAN pcaps enabled

- Artifact dir: `lab/_artifacts/20260429T070245Z-mnt01-mnt01-debug-kcp/`
- Key files:
  - `attempt-1.md` / `attempt-1.json` (same EOF symptom)
  - `natA.lan.pcap`, `natB.lan.pcap`, `natA.wan.pcap`, `natB.wan.pcap`, `wan.pcap`
  - `natA.conntrack`, `natB.conntrack` (shows UDP flow exists; not “completely unreachable”)
  - `daemon-a.log`, `daemon-b.log` (currently only shows “serving LocalAPI”)

## What We Tried (So Far)

- Added extra server-side logging around acceptor stream handling, so we can differentiate:
  - `AcceptStream` fails (yamux / TLS / KCP layer)
  - or `serveAcceptedShellStream` fails (hello / decl / stream-open business checks)
  - Code: `internal/pocacceptor/acceptor.go` (logutil.Infof in stream accept loop)

However, current artifacts do not include these log lines, suggesting one of:

- The updated `miopunch` binary was not pushed into the lab VM before the debug run, or
- The failure happens before the acceptor reaches those code paths, or
- Logging is not routed into the captured `daemon-*.log` as expected.

## Local Dependency Source Checkouts (Avoid Web Search)

To reduce “search the web” loops, we checked third-party sources directly by cloning the exact versions used by this repo into `/tmp`:

- QUIC: `/tmp/miopunch-deps/quic-go` (module: `github.com/apernet/quic-go`, version `v0.59.1-0...`, commit `db4786c77a22`)
- KCP: `/tmp/miopunch-deps/kcp-go` (module: `github.com/xtaci/kcp-go/v5`, version `v5.6.71`)

These checkouts are for code-reading / root-cause confirmation only.

## Working Hypotheses (Need Confirmation)

- KCP path may still be receiving non-KCP UDP packets during/after traversal. If they reach `kcp-go` they can poison the accept path (even if we filter conv=1), leading to early close/reset behavior that shows up as dial-side EOF.
- KCP accept-side TLS handshake or yamux session setup fails and is silently dropped (current code `continue`s on errors in several places). Without logging, dial side only sees EOF later.

## Next Steps (To Break the Loop)

1. Ensure the lab VM runs the updated host-built binaries:
   - Always use `./lab/host/labctl mnt01-smoke` (it runs `push-bin`), or
   - If running `labctl guest ... mlab-mnt01-run` directly, first run `./lab/host/labctl push-bin`.
2. Add *minimal* error logging (or event emission) for KCP accept-side failures:
   - `kcpSessionListener.Accept`: log TLS handshake / yamux setup errors (today they are dropped via `continue`).
   - Keep logs operator-oriented; avoid high-volume spam.
3. Re-run only the KCP case (or smoke) and confirm artifacts include the accept-side failure reason.

## Resolution Update

Confirmed on 2026-04-29 with a fresh host-built binary.

The root cause was in the dialer path (`internal/task/poc_dial.go`): after
`connectivity.Attempt` selected the UDP path, the code set `gather.UDP4Conn` /
`gather.UDP6Conn` to `nil` before selecting the corresponding QUIC/KCP socket
owner. The later owner lookup compared `attemptRes.Conn` against those now-nil
fields, so it never found the existing `KCPOwner` / QUIC transport.

For KCP this meant the dial side fell back to raw `dataplane.DialSession` while
the `KCPOwner` still owned the same UDP socket. When `dialPeerStream` returned,
its defer closed the still-referenced owner, which closed the UDP socket backing
the just-opened KCP session. The caller then reached `CapabilityHandshake` and
read EOF from the logical stream.

Fix: capture `attemptUDP4` / `attemptUDP6` booleans before clearing gather
fields, and use those booleans for owner selection and ownership transfer.

Validation:

- Focused KCP case:
  `20260429T073613Z-mnt01-mnt01-debug-kcp-fixed`
  - `attempt_path=punching_ipv4`
  - `hello=ok`
  - `ping=ok`
- Full smoke:
  `20260429T073746Z-mnt01-smoke-aggregate`
  - `summary: pass=8 fail=0`
