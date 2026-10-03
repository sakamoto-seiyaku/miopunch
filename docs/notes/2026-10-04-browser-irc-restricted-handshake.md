# Browser IRC restricted-handshake budget experiment

2026-10-04 Asia/Taipei. This is the independent IRC version of
[replan step 2](2026-10-03-browser-webrtc-replan.md), following the
[qualified slow MIR1 carrier](2026-10-04-browser-irc-datagram.md).
The measured profile is **unavailable for restricted A**. Neither diagnostic nor
product qualification passed; IRC step 6 stops at this prerequisite. MQTT and
relay/RTC work continue independently. A runner `passed` means that it measured
and cleaned up the expected outcome; `available: false` is the qualification.

## Experiment and implementation boundary

The owned pinned Ergo TLS/WSS fixture retains fakelag and the same explicit
native/WSS/channel mapping. MIR1 remains shared, with 600 ms pacing, 128-byte
fragments, 8-second assembly TTL, 1.5-second queue admission age and at most
2,048-byte datagrams. No QUIC Initial is reduced below 1,200 bytes. Facility
logical ID, native origin and WSS origin each disable the complete trusted alias
before a socket opens; custom routes retain the facility service tag.

[Browser IRC](../../browser-probe/src/irc.rs) now implements the same thread-affine
SendWrapper iroh CustomTransport architecture as MQTT and publishes/withdraws
its qualified addresses. [BrowserNetwork](../../browser-probe/src/network.rs)
accepts explicit trusted `irc: [{facility, profile}]` and contact routes. Its
gate retains the invitation expected-contact identity, existing admission ALPN,
restricted semantics and empty business custom-address allowlist. No parallel
identity protocol, default facility, UI, approval or material delivery is added.

The normal browser budget remains 12 seconds, ordinary native contact 10 seconds
and maintenance 2 seconds. Each ordinary public manual/precode request starts a fresh monotonic caller
budget; retaining the SDK object or an expired IRC facility cannot consume a later
MQTT caller's budget. Startup is its own bounded operation. The explicit IRC-only
fixture instead starts one caller deadline before startup, then passes its actual
remaining time through readiness, authenticated handshake and request. The existing five-second
admission stage agreement is capped to that remaining budget, never extended.
The explicit IRC-only fixture diagnostic can choose up to 45 seconds; it cannot
return a business handle and does not change the normal entry's timeout.
Native fixture setup has its own single 10-second startup bound and uses the
existing explicit `Endpoint::custom_addrs()` seam. `Endpoint::addr()` intentionally
omits signaling candidates; waiting on it was a retained setup failure.

The native fixture uses NativeNetwork's durable node identity, admission gate
and existing manual request handler over IrcTransport::from_facility with normal
CA/hostname verification. Its saved signaling origin is authoritative for the
resource policy. The native saved parser still does not install a TLS/channel
facility mapping for normal product startup. Advancing that integration or IRC
step 6 is inappropriate after this measured stop branch.

## Actual evidence and interpretation

Fresh native/WASM builds and wasm-bindgen precede both final real runs. Durable
[qualification](../evidence/browser-irc-restricted-handshake-2026-10-04/qualification.json)
and [validation](../evidence/browser-irc-restricted-handshake-2026-10-04/validation.json)
retain source hashes, receipt hashes, counters and scope. Raw receipts are linked
there; temporary private keys and NSS profiles are removed by fixture cleanup.

In the 45-second diagnostic, normal native TLS 1.3 and actual WSS 101 succeeded.
QUIC/TLS authenticated the invitation contact, but the existing admission stage
agreement failed after 5,012 ms. The caller ended after 16,088 ms with local
admission rejection. No A was returned, no application request was accepted and
no business handle or admission materials were delivered. Authentication alone
is not a completed restricted A. The saved browser status uses its existing
`AwaitingApproval` label even after an unanswered attempt; only the returned
request result and native accepted-request table prove an actual exchange. Here
the result is null and that table is empty.

The browser emitted ten Initial datagrams, 12,000 bytes; QUIC's authenticated
connection reported ten CRYPTO frames, two lost packets and 2,400 lost bytes.
Browser accepted datagrams totaled 13,729 bytes with queue peak five/drop seven;
native accepted 7,276 bytes with peak eight/drop ten. Accepted bytes include
later-expired queue entries and are distinguished from complete packet counts
and actual WSS frame totals. The largest received IRC line remained 421 bytes.
Repeated Initial emissions are measured directly; they are not mislabeled as
an exact retransmission counter unavailable from the public stats seam.

At the actual 12-second budget, contact TLS authentication left only about
932 ms for stage agreement. The caller ended at 12,001 ms without A or application
exchange. Browser emitted twenty fragments/two complete datagrams, accepted
12,126 bytes, received 1,250 bytes and dropped four queued datagrams. Native
received two and sent two complete datagrams; its queue peak was eight/drop
three. Scheduling milliseconds are recorded; they do not extend the deadline.

These runs qualify the **negative outcome and bounded cleanup**, not a usable
IRC A. Wrong-contact/original-versus-other-requester admission isolation was not
advanced on this unavailable profile: a successful original A is its prerequisite.
Existing identity/admission protocol remains intact; MQTT qualification does not
substitute for those missing IRC stories. No claim of IRC step 2 or 6 success
is made, and no product timeout/load/TTL change is justified by this result.

## Retained SDK caller regression

Review found that using the SDK constructor timestamp for every request also
exhausted a healthy MQTT retry in mixed configurations. The real failing public
seam had a 11.888-second IRC attempt, no native request, then the same browser's
MQTT retry ended after 8.47 ms. The fix gives every ordinary manual/precode caller
a fresh 12-second deadline. Only the explicit IRC fixture seeds its request with
the already-started startup-inclusive deadline; object age is never a public
caller deadline.

The actual mixed regression holds one browser identity and one native endpoint
with both owned facilities. MQTT qualification stays alive while only its QUIC
publications are dropped for the original attempt; the paced IRC attempt fails
without an application request. The same retained browser then uses MQTT and
returns AwaitingApproval, leaving exactly one unapproved native request, unchanged
membership and no saved authorization/secret. A separate first-call test idles the
SDK for 13 seconds until IRC has retired, then its first MQTT admission succeeds
within a fresh caller budget. These are MQTT isolation regressions, not IRC A or
IRC6 qualification. Their exact timings/source hashes and zero cleanup receipts
are in the durable qualification.

The native fixture now lets the caller consume its buffered unapproved reply and
close A under the same server deadline. A retained intermediate failure proved
that closing immediately after queuing that byte could preempt its delivery.
There is no added application acknowledgement or timeout extension. Shared owned
IRC fixture utilities and a concrete browser receive-metadata copier remove the
reviewed duplicated scaffolding; independent runner entry points remain.

## Retirement and retained failures

A terminal native `IrcTransport::close()` withdraws route eligibility before
QUIC graceful drain, cancels its driver and serializes with late ready publication.
The public slow-send/PONG/close test first failed because this explicit retirement
seam was absent, then passed while retaining the datagram owner. Browser terminal
retirement clears its timer, route, outgoing/RX queues and assemblies before drain.
No late event can republish the retired generation.

Caller elapsed time and cleanup elapsed time are separate. The diagnostic caps
graceful browser QUIC drain at two seconds; if it expires, the runner destroys
its owned browser context, which ends its remaining JS/iroh jobs. Native retires
the carrier, waits at most two seconds for the driver and separately caps QUIC
drain at 500 ms. The runner checks actual WSS close events, zero Window intervals,
zero native driver/process/queue/assembly state and finally removes the owned
container. Entire fixture/protocol/cleanup took 21.416 and 18.001 seconds of the
60-second outer process-group budget, with ten seconds reserved for cleanup.
A failed graceful drain is reported, never relabeled graceful closure.

Retained earlier failures are setup-only Docker denial, use of the non-signaling
`addr()` view, and an attempted watcher wait on that same wrong view. The first
real handshake also timed out natively after 36.1 seconds, and its runner guard
caught unbounded graceful retirement. A later genuine local admission rejection
exposed native QUIC drain keeping the carrier active. Their raw receipts and
cleanup outcomes remain referenced in validation; none is erased or converted
into positive TLS/handshake qualification.

## Reproduce the bounded stop branch

```sh
rtk proxy python3 scripts/check_browser_irc.py --case irc-restricted-handshake --caller-budget-ms 45000 --output /tmp/mio-irc-diagnostic-new
rtk proxy python3 scripts/check_browser_irc.py --case irc-restricted-handshake --caller-budget-ms 12000 --output /tmp/mio-irc-caller-new
```

The independent runner avoids editing the main browser-path dispatcher while
other agents work there. It uses the unchanged shared private NSS/CA helper,
owned random nicknames/channel, no public traffic, no certificate-bypass flags
and no global trust changes. Compilation is separately bounded/recorded and
uses two jobs. Existing datagram mode remains available for affected regression.
