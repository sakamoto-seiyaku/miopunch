# Browser admission, LogSync and recovery through an official public relay

[MIOPUNCH-65](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/a6ab18c8-763d-4ec4-8f3f-f5dc4bc96da0)
qualifies the existing `BrowserNetwork` against number0's actual public relay.
The prior [browser admission](2026-10-02-browser-manual-admission.md) and
[control replica](2026-10-02-browser-control-replica.md) results remain their
original self-hosted qualification. This check adds normal public TLS trust;
it does not change admission, signatures, SCRAM, LogSync or IndexedDB behavior.

## Service and trust

The [official relay documentation](https://docs.iroh.computer/add-a-relay)
identifies n0's public relays as development/testing resources. The
[current official source](https://github.com/n0-computer/iroh/blob/main/iroh/src/defaults.rs)
and [pinned source](../../vendor/iroh/src/defaults.rs) list the same four
production hostnames. These sources were checked on 2026-10-03. The selected
resource was `https://aps1-1.relay.n0.iroh.link./`; the actual browser connection
was `wss://aps1-1.relay.n0.iroh.link./relay`, with HTTP 101 and `iroh-relay-v2`.
The root HTTPS path returning 404 is a service/TLS observation, not evidence
of member communication.

Chrome for Testing 153.0.8010.12 used normal certificate validation:
`ignoreHTTPSErrors: false`, no certificate-ignore argument or test CA.
The native fixture likewise received no custom CA. Its endpoint clears direct
IP transports and uses only the selected relay with the existing native
resource policy. Public relay traffic retains the real end-to-end iroh
TLS/QUIC and membership gates.

The client's origin is an ephemeral `http://127.0.0.1:PORT` secure local
context. Its HTTP server only distributes the existing HTML, JavaScript and
WASM; it never approves members, holds node material or proxies traffic.
Public WSS remains fully certificate-validated. The browser's independent key,
authorization, admission secret and control records live in its private
IndexedDB profile; the separate native node uses its normal private store.

## Qualified vertical slice

The test first drives an actual manual request and one native approval.
`PendingMaterials` precedes `complete_admission`; success requires both saved
authorization and saved secret. A controlled member message is echoed over the
real admitted connection. The native fixture then authors the existing signed
control-record example with a nonempty body. Browser `sync_replica` runs real
p2panda LogSync, reports its native source and three-record manifest, and saves
the original signed record/body. Refresh restores the same key, both admission
materials and exact record and permits another member exchange.

Only after this first complete check, a new page explicitly disables the
selected relay. Startup rejects with `temporarily unreachable: no allowed
browser relay`; business controls stay disabled. Opening with `{}` then
confirms that the disable was persisted. Both attempts access only the static
origin and make no WSS or alternate-facility request. Explicitly restoring the
allowed resource and reopening recovers the same identity, record and member
communication. Native record count, members and request count remain unchanged,
and only the original approval occurred. Closing a page never authors a leave.

Chromium CDP records actual WSS handshake status, protocol and per-direction
frame/byte counts without recording frame payloads. Native observations use
`Connection::paths` at admission, completed replica and business exchange:
every path is the declared official relay, with a selected path and nonzero
traffic in both directions. Combined with the verified saved record and
application reply, these observations qualify actual relayed member data,
rather than treating a socket opening as success.

## Reproduce and scope

```sh
rtk proxy python -B scripts/check_browser_public_relay.py --output /tmp/mio65-check --checks
```

Use a new output directory. The runner builds current native/WASM code and
uses the existing wasm-bindgen 0.2.129 and pinned Playwright Core installation.
`--relay` selects exactly one of the four declared official URLs; it never
automatically retries another facility. `--skip-build` explicitly reuses a
build. `PLAYWRIGHT_CHROMIUM_EXECUTABLE` and `--wasm-bindgen` can select installed
tools. The existing MIOPUNCH-51 replica scenario is a separate affected
regression with its original self-hosted TLS fixture; its test CA is never
used in the public scenario.

The public check uses a unique node directory/profile and ephemeral page-server
port, closes its own native fixture, and removes its private state. The Python
driver owns the temporary directory and limits the scenario process group so
an outer timeout also cleans its owned fixture state. MIOPUNCH-64's Android,
Docker and node resources are untouched.

The checks ran while MIOPUNCH-64 was concurrently developing shared native/core
paths. They build the current tree rooted at `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`,
including that concurrent work; they do not establish that an older cached
runtime or the pre-64 core has the same result. No shared core changes were
needed for this browser ticket. The only Rust change is bounded fixture startup
and public path observations; `BrowserNetwork` itself required no adaptation.

See [the evidence index](../evidence/mio65-public-browser/README.md) for actual
access times, origin, commands, failures, short checks and independent review.
This qualifies one Chromium/Linux foreground scenario and one actual public
relay at the recorded times. Public availability remains best effort. Public
website deployment, other browsers, browser direct UDP/WebRTC, background
residency and a service SLA are outside this result. Already verified unchanged
slow NAT matrices were excluded under the user's standing instruction.
