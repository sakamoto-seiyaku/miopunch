# M76 current runtime evidence — 2026-10-06

> The remaining API-available RTC network-failure qualification is independently verified in the [later follow-up record](../mio76-network-failure-2026-10-06/README.md). This earlier evidence and its known-gap wording remain readable as captured historical material.

This evidence set records three actual HostNative + PixelBrowser runs through
the current M76 controller:

- API-missing RTC capability, no permitted data path, then restored permitted
  Relay (`missing-answer`).
- PeerConnection creation denied while a permitted Relay still carries member
  traffic (`pc-denied`).
- DataChannel creation denied while a permitted Relay still carries member
  traffic (`dc-denied`).

All three actual receipts passed with exit 0 and are preserved at their
immutable local paths. They prove independent identities, durable admission
materials, current member gates, fresh ACKs, `rtcRole: None`, selected
permitted Relay, real four-byte PING/PONG in both directions, and same-M
continuity. The API-missing run additionally proves typed
`DataPathUnavailable`, zero MQTT QUIC business bytes, no ordinary handle, and a
new attempt after Relay restoration. The denied-constructor runs deliberately
do not claim the API-missing no-data/restoration branch.

The real MQTT contact carrier retained zero SDK property callbacks in these
three public stories. The configured permitted Relay retained four essential
terminal-close callbacks while CLOSING, with zero buffered bytes and unchanged
traffic through the 100 ms observation. The public contract records this as
pending; graceful Relay completion, all terminal SDK task retirement, and the
Relay receive queue remain unobserved/unknown. Owned browser tab/store/ADB
mappings/listener and Native process cleanup completed naturally and are
reported separately from protocol-graceful Relay completion.

The runs reuse the frozen Native receipt/ELF (`588f…` / `a916…`) and Browser
receipt/package (`1a77…` / `ef03…`). The full exact hashes, command arguments,
caller and cleanup timings, capability counters, roles, path category, ACKs,
cleanup facts, and receipt provenance are in [results.json](results.json).

The reviewed source allowlist is [source-freeze.json](source-freeze.json), and
the two concentrated read-only reviews are [review-spec.md](review-spec.md)
and [review-standards.md](review-standards.md). The reviews found zero required
code fixes. The standards review recorded two optional Fowler judgements.

The public evidence intentionally omits raw private RPC/cleanup journals,
compiler-input archives, SDP/ICE/exporter material, identity or admission
values, device serials, OS routes, and private credentials. Local first-failure
receipts and causal feedback remain referenced by path and SHA-256 in the
structured results; none are copied here. A post-publication privacy check
compares the published text against identity, admission-material, and route
values collected from all three actual cases without printing those values. It
passed after scanning 94 local private files and comparing 17 sensitive string
values, with zero collisions and an exact match to the four-file source
allowlist.

Known limits remain explicit. `physicalWanQualified` is false. API-available
RTC network-establishment failure followed by successful permitted Relay is not
qualified here and remains a planned M77 composition. The historical explicit
RTC-disabled R83 branch is retained only as an accepted unaffected reference;
it is not counted as one of these three current stories. No old 43-stage replay
or broad matrix rerun was used. M76 remains in progress until Root completes
the remaining qualification, review, commit, and Plane work.
