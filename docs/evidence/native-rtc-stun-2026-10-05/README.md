# Native RTC public STUN observation — MIOPUNCH-71

2026-10-05. This is a gathering-only qualification of the existing native RTC driver. It is not a two-peer QUIC/SDK member-flow, new unified-wire, or physical public NAT data qualification. The host upstream gateway uses WARP; the observed Cloudflare mapped address is not evidence of a no-VPN path.

The original wildcard driver timed out at 12,001 ms. The socket bound to the routed host interface sent a real STUN Binding request and received a matching response, while five additional library-enumerated interface sockets had no responses. Their outstanding transactions prevented the aggregate gathering-complete event. A single-interface differential, changing only the binding, completed in 205 ms with actual srflx candidates. Independent raw requests on wildcard and physical bindings both received matching responses in about 188 ms.

The correction adds `RtcIceProfile.local_bind_addresses`, a bounded explicit list of native IPv4 socket addresses. Ports remain ephemeral. An empty list retains library wildcard behavior. It does not change `candidate_addresses`, which remains a publication/peer-candidate filter, does not choose a transport at the business API, and does not discover or hardcode a physical interface. The formal native factory consumes this profile. The opt-in test uses that factory's exact builder and public driver operations, with a passive runtime wrapper delegating real Tokio UDP sockets and emitting only STUN header metadata.

The profiled public run completed in 190 ms: request at 3 ms, matching response at 190 ms, gathering-complete and srflx candidates. Explicit `pc.close()` returned in less than 1 ms; process socket count returned from 5 to baseline 3 at the independent 301 ms observation, and the owned RTC socket tracker returned to zero. Three silent-observer retries exhausted their 150 ms absolute caller budget at 151 ms, returned from close in less than 1 ms, and retired all seven owned socket wrappers in 5–6 ms. This preserves the existing product deadline and library transmission schedule; no public send-rate increase or TURN is introduced.

The default wildcard profile remains **unqualified in this environment**. Library `webrtc 0.21.0` does not expose a STUN gathering transaction timeout through `SettingEngine`; its ICE health/check/acceptance timers are different mechanisms. The gatherer's terminal non-message event branch only logs and does not retire the remaining client. This investigation does not add a library rewrite or a new trickle protocol. Multiple explicit interfaces can still have the same unresolved-observer problem; callers must supply their already selected local adapter binding for this qualification.

## Reproduction

Run with actual network permission, serially:

```sh
rtk proxy cargo test --features rtc-probe --lib public_stun_gather -- --ignored --nocapture --test-threads=1
MIO_RTC_STUN_BIND=<actual-local-IPv4> rtk proxy cargo test --features rtc-probe --lib public_stun_gather -- --ignored --nocapture --test-threads=1
rtk proxy cargo test --features rtc-probe --lib silent_observer_deadline_and_retry -- --nocapture --test-threads=1
rtk proxy cargo test --features rtc-probe --test rtc_ice_profile
```

The public observer defaults to the same numeric observer in the retained Y700 failure, `stun:74.125.250.129:19302`; `MIO_RTC_STUN_OBSERVER` permits an explicit replacement. It is a diagnostic opt-in, not a default facility decision. `receipt.json` retains exact commands, raw command exit values, logs and final source/library/artifact hashes. Early diagnostic iterations are preserved as first failures; their overwritten build artifact is explicitly unavailable, rather than incorrectly given the final binary hash. The final default/profile contrast uses the recorded final artifact.

Gathering has one native offerer and a STUN observer, no second authenticated node or selected data path. Identity and exporter match fields are therefore null/not applicable; they are not asserted true. No M, route lease, QUIC pump, business queue, contact listener, device forwarding, or helper service is created. PCs/channels/driver sockets are owned by each test, closed and observed after close. The local silent-observer socket is closed by its test. No process was forcibly terminated. Logs contain candidate address/type/port and STUN request/response metadata, but no raw SDP, transaction ID, exporter, credential, private key, or network secret. The newer unified SDK must repeat its complete public bootstrap qualification in a later slice.
