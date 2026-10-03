# Browser/native synthetic RTC channel, step 4

2026-10-03 15:51:33–15:51:36 UTC. The runnable step-4 experiment in the
[replan](../../notes/2026-10-03-browser-webrtc-replan.md) passed on Linux,
Chromium 153.0.8010.12, native `webrtc =0.21.0`, browser `web-sys 0.3.106`,
Rust 1.98.1, and wasm-bindgen 0.2.129. This is a test profile and candidate
qualification, not formal library adoption, a QUIC bridge, or an original MIO
session changing its data path.

Run from the repository root with the installed harness dependencies and pinned
wasm-bindgen tool. Cargo downloads are separate; builds use the recorded lockfile.
The output directory must be new:

```sh
rtk proxy python3 scripts/check_browser_rtc.py \
  --target-dir /tmp/miopunch-rtc-s4-target \
  --output /tmp/miopunch-rtc-s4-new-run
```

The default owned fixture creates a temporary localhost CA, leaf key, and private
NSS database. Chromium sees this database only inside its own bwrap mount
namespace; native trusts that CA for this fixture. Chain, time and hostname
verification remain enabled. Host trust and HOME are unchanged. The private-trust
helper is shared with step 1. No certificate, private key, network admission
secret, SDP, ICE password or TLS exporter is saved in evidence.

Two independently identified nodes complete the existing manual-admission
fixture and establish a current, admitted member M over that HTTPS/WSS relay.
SDP, embedded ICE candidates and DTLS fingerprints travel in a protected QUIC
stream on that M. Its envelope checks peer identities, network, secret generation,
random attempt and a digest of an exporter scoped to that context. This adds no
parallel identity or password protocol. Both endpoints explicitly confirm the
channel profile over that private stream before sending synthetic data.

Actual [channel receipt](channel.json):

| Observation | Browser | Native |
| --- | --- | --- |
| Profile | negotiated=true, id=0, unordered | negotiated=true, id=0, unordered |
| maxRetransmits / maxPacketLifeTime | 0 / null | 0 / null |
| maxMessageSize | 65536 | 65536 |
| Matched synthetic message | 1200 bytes | 1200 bytes |
| TX / RX payload queue peak | 1200 / 1200 bytes | 1200 / 1200 bytes |
| ICE / DTLS / SCTP | connected | connected |
| Selected pair | host / host, UDP | host / host, UDP |

Both selected addresses were the same host's LAN address `192.168.4.5`, with
independent UDP ports 43416 and 46316. This is same-host/LAN API interoperability,
not an across-device or NAT success-rate result. No STUN or TURN URLs were
configured on either side. A peer-reflexive candidate in an earlier receipt was
also direct; the test rejects relay candidates instead of assuming direct means
host-only.

Native reported open after 269ms; the complete protocol process took 2.971s,
plus 0.595s of fixture provisioning. These are observations, not an SLA or a
performance comparison. The browser's actual data-channel stats recorded one
1200-byte message sent and one received before a separate four-byte DONE control
acknowledgement. Both adapters preserve message boundaries. Browser copying
observations distinguish the generated WASM memory view from the explicit
JS-to-WASM receive copy; native records Vec-to-BytesMut and Bytes-to-Vec copies.
Browser/library internal copies and total resident memory were not instrumented.

Application TX/RX queues each cap payload at 64KiB and message count at 55,
with a 1200-byte maximum. Native additionally sets a 64KiB channel send limit
and 64KiB SCTP receive limit; browser's single send pump checks bufferedAmount
plus the next whole message against 64KiB. These are application/buffering
bounds, not a claim that the complete browser or library uses only 256KiB.
The real browser fault fixture refused another message at a 64800-byte queue
peak, cleared both queues on cancel, and ignored an injected late open event.
The public association tests verify that close wakes a full sender and empty
reader, terminal retirement cannot reopen, and tiny messages cannot grow queue
metadata without a message-count bound.

Native socket count was 6 before RTC and 6 after its owned driver closed; both
queues were empty. The browser closes its channel and peer connection and
removes callbacks on drop. The node and owned relay exited with code 0; final
fixture cleanup took 197ms. The browser attempt has a 20s deadline, native open
uses a 20s deadline, the harness reserves cleanup inside 60s, and the outer
runner terminates its owned process group on expiry. No background application
pump is spawned. Fixture provisioning has a separate 15s protection within the
60s attempt budget. A concurrent call on the same M is rejected before a second
RTC object is created; the real run verifies this overlap refusal. Normal and failure receipts retain cleanup facts.

The first browser-offered DCEP profile failed its strict native attribute check:
[receipt](dcep-getter-failure.json). The published `rtc 0.21.0` incoming-channel
`accept()` wrapper discards the reliability type and sets both reliability
metadata fields to None, although the underlying data-channel config retains
its wire fields. This demonstrates a getter limitation; it does not alone prove
wire reliability was wrong. The fixed package identity and source hash are in
[source-receipt](source-receipt.json). The passing profile uses the standard
[out-of-band negotiated option](https://www.w3.org/TR/webrtc/#dom-rtcdatachannelinit-negotiated)
with id 0 and the explicit private-M agreement. Qualification is limited to that
profile, the browser RTC offerer, and the observed roles. It does not qualify
incoming DCEP metadata, other offerer roles or a second adapter. No str0m or rtc
source fork was added.

[An intermediate negotiated-profile receipt](fixture-lifetime-failure.json)
matched both payloads but failed to deliver the final control receipt because
the test handler dropped M too early. The handler now retains that original M
until the caller closes it; the passing command validates the corrected lifetime.

Two approved public-relay attempts stopped before RTC:
[material-delivery interruption](public-setup-material-failure.json) and
[first-request path failure](public-setup-path-failure.json). Both had normal
WSS101 but failed at different existing admission preparation points. The owned
TLS fixture isolated that variable without changing product timeouts or
weakening member qualification. They are not RTC failures or evidence that the
public facility is permanently unavailable.

Build commands/times are in [runner.json](runner.json), current library and
short seam checks in [validation.json](validation.json). Transitive rtc
requirements raised rcgen from the old lock's 0.14.7 to 0.14.10 and async-trait
from 0.1.88 to 0.1.92. rkyv's additional SocketAddr PartialEq required explicit
SocketAddr types in two existing test files; no slow NAT matrix was adopted.

For step 5, reuse the bounded queue/profile and control envelope within the
existing admitted Connection seam. The present `rtc_channel_probe` consumes
synthetic data and retires RTC; it returns no production path lease and does not
forward QUIC datagrams. Keep the original M for the bridge/probe and qualify
fresh ACK, exact path allowance, cancellation and current authorization there.
The independent runner can be incorporated as `check_browser_paths.py --case
rtc-channel`; no formal UI, IWA, TURN facility or library decision is introduced.
