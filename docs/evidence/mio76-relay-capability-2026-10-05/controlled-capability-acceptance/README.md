# Controlled MIO76 SDK acceptance supplement

Four changed cases passed through the real host Browser SDK and the independently
admitted, reopened Native SDK. They use an owned TLS iroh relay, owned TLS/WSS MQTT,
a cached local Mosquitto container, and a temporary CA/NSS browser profile with
certificate verification enabled. No public facility, Android device or STUN probe
was contacted. The original 12-second member caller and Native 6-second relay
validation limits remain unchanged.

| Case | Actual capability / owned runtime objects | Member outcome |
| --- | --- | --- |
| Policy disabled | Support unknown; permission/effective false; PC/DC created 0/0 | New member session, 865.4 ms, 39-byte request and echo |
| PeerConnection constructor denied | Support false; permission true; effective false; PC/DC created 0/0 | New member session, 787.8 ms, 40-byte request and echo |
| DataChannel creation denied | Support false; permission true; effective false; real PC created/closed 1/1, DC created 0 | New member session, 740.1 ms, 40-byte request and echo |
| All Browser data relay permissions disabled, then restored | Policy disabled; MQTT remains ready; no ordinary member handle; DataPathUnavailable in 10.2 ms | MQTT QUIC sent/received 0/0, Native business events 0; identity, saved materials and running Complete state preserved. Restoring permission produces a different member session in 774.4 ms, with a 43-byte request and echo |

The no-data story first establishes an ordinary session in 775 ms before changing
the relay permission. Both successful sessions have independently compared peer
identities, actual current member gates, strict `miopunch/session/member/1`,
capability version 1, Browser RTC role None, fresh authenticated ACKs at both ends,
and a selected ordinary relay path. The Native business event matches the original
member stable ID and exact request byte count before private identities are removed
from readable safe receipts. Raw archives preserve process-local bootstrap table
correlation keys byte-for-byte, while readable safe receipts replace those keyed
tables with observations. The Browser returns the exact requested echo. The restored session
has a different observed Native member stable ID; those IDs remain private.

Each case closes its owned Browser member/network and context, observes PC/DC and
WebSocket retirement, ends Native/relay processes and readers without force,
records actual Native SDK closed events with RTC associations/pumps and MQTT queues
at zero, and observes Native socket counts 4 → 4 → 4. The local service socket set is
empty, owned servers close, the named container removal exits zero, and temporary
certificates/NSS/profile directories are deleted. A subsequent `/proc` check finds
no process using any of the four exact owned fixture directories. Relay WebSocket
1006/unclean close observations remain in receipts where they occurred; socket
retirement does not imply every transport completed a graceful WebSocket close.

The SDK artifacts are the immutable private diagnostic build
`/tmp/mio76-ack-private-probe-build-20261005`, with 276 actual selected Native inputs
and 186 actual selected WASM inputs verified against the frozen source projection.
Native member ELF SHA256 starts `1e26fe54`, relay fixture ELF `3dc9b83b`, and WASM
`111150dc`. Artifact, helper and selected projection hashes remain unchanged in all
four runs. Each outer receipt records exact differences from the current main
source; this supplement does not qualify the current main build, Android or public
NAT behavior. The already successful controlled API-missing baseline was reused as
evidence and was not rerun. Earlier phone failures and the physical disabled pass
remain untouched.

[receipt.json](receipt.json) and [archive-audit.json](archive-audit.json) pin every
raw receipt and log, the copied owned controllers,
build/source/dependency metadata, cleanup observations and current-main deltas.
The first outer preflight failed before any SDK or service started because it used
the metadata platform key `wasm` while the canonical inventory names it `browser`.
Its source and failure record are preserved under `raw/first-preflight-failure`;
only the owned metadata key was corrected. All actual SDK runs exit zero. Independent
review and ticket closure remain pending.

Readable case receipts are under `safe/`; byte-exact compressed originals are under
`raw/`. Controllers are exact copies of the owned temporary variant.
[source-input-map.json](source-input-map.json) retains the frozen broad source input
map and actual selected input lists.
[baseline-api-missing-reference.json](baseline-api-missing-reference.json) hashes
the existing baseline without copying its SDK materials or rerunning it. This
subtree includes no executable ELF, certificates, keys, browser profile, durable
membership material or secret. Current phone receipts, including the separate
Native startup observation subtree, were not edited.
