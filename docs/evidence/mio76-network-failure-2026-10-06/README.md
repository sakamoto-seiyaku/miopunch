# MIOPUNCH-76 network-failure follow-up — 2026-10-06

This packet publishes one current controlled SDK run for the remaining M76
qualification requirement: when RTC APIs are available and permitted but an
RTC candidate cannot be established, the coordinator reports the candidate
failure while preserving the already authorized relay session. The run uses
Host Native and Host Browser, with a local owned loopback data relay and
public EMQX admission/contact over normal TLS.

The run is linked to [MIOPUNCH-76](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/a58dd0f6-99ba-4f26-a033-5a128c2f647a) and its controlled runtime receipt is labelled
[MIOPUNCH-77](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/69773d76-0b92-49a2-8a56-cb323ee6272d). The earlier M76 publication remains at
[mio76-current-runtime-2026-10-06](../mio76-current-runtime-2026-10-06/README.md);
its source freeze and two concentrated reviews are linked from
[provenance.json](provenance.json) and are unchanged.

The immutable runtime receipt passed with command exit 0. Admission and the
ordinary member path completed with matching endpoint identities, current
member gates, fresh acknowledgements, and the selected permitted data relay.
The active RTC improvement attempt created one new PeerConnection and one new
DataChannel beyond the retired startup probe, then returned
`DeadlineExceeded` after 2004.835 ms against its unchanged 2000 ms caller
budget. The original member M, stable identity, local binding and stream were
unchanged; an independent PING received PONG after the candidate failure.
There was no offline result and no replacement member identity.

The public qualification flags are:

| Result | Value |
| --- | --- |
| `networkFailureQualified` | `true` |
| `m77Qualified` | `false` |
| `physicalWanQualified` | `false` |
| data path during the run | permitted local owned relay |
| admission/contact carrier | public EMQX, normal TLS |

`physicalWanQualified:false` is deliberate. This run does not establish a
physical WAN path, a public data relay, or NAT traversal success. It is a
controlled coordinator-fallback observation whose relay remains healthy while
the RTC candidate attempt fails.

The RTC object counts exclude the startup probe. The baseline had one retired
PeerConnection and one retired DataChannel. The active attempt added exactly
one of each; both had zero open objects immediately and at the +100 ms sample.
The whole candidate call remained bounded by the 2000 ms saved budget; the
observed caller duration was 2004.835 ms and the reported failure class was
`DeadlineExceeded`. The original stream exchanged its post-failure PING/PONG
successfully through the same permitted relay.

Cleanup was natural and separately observed. Browser SDK close/free returned
zero owned member handles, zero active RTC associations and pumps, closed and
freed the network, deleted owned storage, and retired the owned tab and
process group. Browser PeerConnection/DataChannel counts were zero at both
samples. One MQTT socket remained in `CLOSING`; its late buffer and SDK
callback counts were zero and traffic was unchanged at +100 ms. Graceful
protocol close and retirement of every terminal SDK task remain unknown and
are left pending. Native cleanup was not forced: maintenance stopped, its
task joined, application stream handlers and RTC pumps were zero, and the
owned process group went from two processes/seven sockets before close to
zero processes/zero sockets immediately and at +100 ms.

The public structured data is in [results.json](results.json). It preserves
the exact command, caller and phase timings, build and runtime pins, result
booleans, cleanup observations, and privacy limits without copying private
operation journals. The immutable source receipt is
`/tmp/mio77-controlled-current-20261006/runtime-streams-first/receipt.json`
with SHA-256
`c08b80a4a54da3255cf31d062dcfdb20bf960e258c6567e75fd045713c320c16`. Its
63-record private operation journal remains at the same immutable run root,
with SHA-256
`805f309bf5bd2784dc383f01e36d6793aef3bbf9c7b11b7a9b3c7cf8014e89d5`; the
journal is referenced by path and hash only.

The scoped follow-up review is copied as
[review-spec-followup.md](review-spec-followup.md). It reports zero remaining
findings and zero required code fixes for this one M76 gap. It does not approve
the complete M77 goal, physical WAN or public data-relay qualification. Root
still owns the repository commit and Plane closure steps; this packet does not
state that the broader M77 work or the M69 parent is complete.

No old SDK test matrix, replay, or broad runtime rerun was added for this
record. The only runtime input is the immutable controlled receipt identified
above.

The privacy check compared actual private identity, admission, route and
profile values without printing them. It passed with zero public collisions;
the counts and checker hash are recorded in `results.json` and
`provenance.json`. The packet publishes no raw SDP, ICE, exporter bytes,
identity or admission values, secrets, profile contents, device serials, OS
routes, private journals, compiler archives, or temporary credentials.
