# Browser IRC-WSS slow MIR1 datagram qualification

2026-10-04 Asia/Taipei. Authorized implementation of step 3 in the
[WebRTC replan](2026-10-03-browser-webrtc-replan.md). This is a carrier experiment,
not an IRC QUIC/TLS, member connection, RTC, UI, public-service or product-default
qualification. MQTT and RTC remain independent.

The owned pinned Ergo 2.7.0 fixture exposes native TLS and IRC-WSS directly.
It retains fakelag (burst 5, 2 commands per second after the burst), creates two
random independent nicknames and a random private test channel, and disables
channel history. Native rustls checks the CA and `localhost` hostname. Chromium
uses the same ephemeral CA in a private NSS database mounted only in its child
namespace. No host trust changes or certificate-bypass flags are used.

## Contract and qualified profile

The [shared MIR1 module](../../src/irc_datagram.rs) preserves the existing
public target/source route fields, message ID, fragment index/count and hex
payload. It does not replace iroh's authenticated identity or session. Custom
addresses retain `mioirc` and their service tag. Trusted local facility
configuration explicitly maps native and WSS entries to one logical facility
and channel namespace; the namespace participates in its service tag. Disabling
either origin or the logical facility rejects the whole mapping before any
socket is opened. Hostname equality never establishes an alias.

The experimental slow profile is explicit: 600ms minimum between data fragments,
8,000ms assembly lifetime from the first fragment, 1,500ms queue admission age,
30,000ms total carrier attempt, 128-byte fragments and 2,048-byte maximum datagrams.
Profile validation caps pacing at 2,000ms, assembly lifetime at 15,000ms, queue
age at 5,000ms and the whole attempt at 60,000ms. Pending assemblies are capped
at 32, retired/replay records at 128, and outgoing buffering is bounded: browser at 64 datagrams; native uses a
64-slot futures channel, one sender-reserved slot and one active datagram (at
most 66 datagrams). Identical duplicates do not restart the TTL;
conflicting duplicates retire that message ID, expired IDs reject late fragments,
and cancellation retires the complete assembly generation.

The old native entry keeps its fixed namespace and original defaults: 3,000ms
assembly TTL, 500ms queue age and 10ms pacing. The new profile is not silently
installed as a native default. The 12-second browser, 10-second ordinary-contact
and 2-second maintenance caller deadlines are unchanged. A 30-second experiment
budget does not grant a longer product caller budget.

Both adapters publish readiness only after their own 001, 005, JOIN and matching
366. Cancellation, failed registration or socket loss withdraws readiness. Native
ready publication and retirement share one lock; each binding owns its diagnostic
report, so retirement of an old driver cannot stamp a replacement route. Native
TCP/TLS startup also observes cancellation. Native control commands take priority
between paced data fragments; browser PONG writes bypass its paced data queue.
Normal close writes PART/QUIT, retires receive and send queues, and closes the
owned socket. Browser error/deadline paths also stop the periodic task even when
the caller retains the expired adapter; real interval tracking verifies this
before explicit cleanup. Native cleanup writes have a separate bounded 2-second ceiling.

## Reproduction and evidence

```sh
rtk proxy python3 scripts/check_browser_irc.py --case irc-datagram --output /tmp/mio-irc-new
```

The standalone runner avoids concurrent edits to the shared browser-path runner.
After integration, the root may dispatch `check_browser_paths.py --case
irc-datagram` to this entry. Linux dependencies match the previous MQTT fixture:
Docker with the pinned Ergo image, Rust/WASM and matching wasm-bindgen, installed
Playwright/Chromium, bwrap and NSS. Install the existing harness lockfile with
`rtk proxy npm ci --prefix browser-probe/harness`; the qualification uses normal
certificate verification. Compilation is recorded separately; fixture setup,
two complete normal datagrams, one local failure group and cleanup share one
60-second outer attempt; 10 seconds are reserved for process-group termination,
optional log collection and owned container removal. A log timeout never skips
removal or receipt saving. Only owned loopback sockets and containers are used.

Actual normal receipts record each direction's one 1,200-byte datagram, payload
hash/equality, ten fragment indices, observed first/last arrival span, full line
length including server prefix/CRLF, ISUPPORT and readiness, queue age/peak,
assembly expiry/rejection/pending peak and close receipts. Slow 10-fragment
arrival spans exceed five seconds; a public seam regression proves that the
unchanged 3-second default cannot deliver that timing. The local failure group
checks missing/conflicting fragments, pending cancellation, PONG during an active
fragment queue, a short whole-attempt deadline and disabling each facility alias.
It completes no third normal datagram and sends no public traffic.

The final current-source build, wasm-bindgen and protocol run passed. Its
[qualification receipt](../evidence/browser-irc-datagram-2026-10-04/qualification.json)
records normal native TLS 1.3, actual browser WSS101, these complete datagrams and
the full source/artifact hashes:

| Direction | Datagram | First/last arrival span | Maximum full received line |
| --- | --- | --- | --- |
| Browser to native | 1,200 bytes, ten fragments, byte equality | 5,429ms | 421 bytes |
| Native to browser | 1,200 bytes, ten fragments, byte equality | 5,413ms | 421 bytes |

Fixture setup, protocol and cleanup took 15.522 seconds within the 60-second
total. Both normal ends retired their queues/assemblies and actual sockets;
native PART/QUIT writes completed, its driver/process count reached zero and the
owned container was removed. A retained browser adapter expired at its short
deadline with zero actual active intervals before explicit close. PONG arrived
during the first slow fragment interval; the local partial datagram never completed.

The [validation receipt](../evidence/browser-irc-datagram-2026-10-04/validation.json)
records 71 affected native library/IRC/resource tests, one existing ignored UDP
data-cut test outside this increment, workspace/all-targets and WASM checks, and
35 targeted library/datagram tests after review fixes. Both Sol6.1/high review
axes finished with zero remaining findings.

Retained red cases include cancellation rebuilding from late fragments, late005
reviving retired readiness, custom-IRC hostname case bypassing disable, one live
browser interval after deadline, and a log timeout skipping owned-container
removal. The logging red fixture's owned container was explicitly removed; its
green rerun proves independent removal and receipt saving. A lifecycle-only
SIGTERM-ignoring injected worker plus log timeout was retired with its real owned
container in 55.75 seconds; this synthetic process test is separate from carrier
qualification. Raw artifacts stay under `/tmp/mio-irc-s3-*`; temporary certificate
keys are deleted with the fixture and never copied into evidence.

## IRC variants of steps 2 and 6

The next IRC entry is `IrcTransport::from_facility` / `bind_datagrams` on native
and `BrowserIrcDatagram` on the browser. `IrcFacility` and `Profile` are the common
configuration seam. The browser adapter is deliberately a standalone datagram
carrier; it is not yet wired into BrowserNetwork or an iroh CustomTransport.
NativeNetwork's existing saved resource parser also has no new trusted facility
mapping or TLS-default selection.

Before an IRC version of step 2, wire this qualified carrier into the existing
restricted A endpoint/control exchange, retain the invitation's expected identity,
and pass the actual caller's one monotonic deadline through registration, queueing,
fragment transfer and QUIC/TLS. Measure handshake retransmissions and queue expiry;
the two synthetic carrier datagrams do not prove that this slow profile fits a
12/10/2-second caller. If that real handshake misses its budget, declare that
profile unavailable and stop its IRC integration path. Continue MQTT; do not
increase service load or extend product deadlines to hide the failure.

An IRC version of step 6 additionally needs steps 2 (IRC) and 5, the versioned
capability/hook protocol, current-member authorization, original-M association,
fresh RTC path ACK and final signaling-only allowlist. None is claimed here.
No new product ADR, default public facility, QUIC implementation or identity
protocol is introduced by this increment.
