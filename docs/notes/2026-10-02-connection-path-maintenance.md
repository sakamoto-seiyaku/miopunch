# Same-Connection quality selection and explicit failure

MIOPUNCH-47 implements the current
[ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/acdfed6c-5c61-40c6-a9a9-bca18644afa3)
and [parent specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
The already approved seams are independent persisted native nodes, ordinary
Endpoint/Connection/stream interfaces, controlled real carriers and faults.
Research and archived Go/OpenSpec gates did not add product requirements.

## Recorded integration choices

Saved `ResourceConfig.path_selection` contains a `PathSelectionPolicy`:

```json
{
  "preference": "Quality",
  "reevaluation_interval_ms": 1000
}
```

Old resource files omit it and receive these defaults. `Quality` compares
actual noq path RTT with the existing five-millisecond switching hysteresis;
relay and direct have equal default rank. `PreferDirect` prioritizes UDP and the
recognized native TCP data carrier; `PreferRelay` prioritizes native relay.
These are preferences among eligible paths. Existing UDP/TCP/relay resource
restrictions remain decisive, including explicit, cached and lookup candidates.
Unknown custom transports do not acquire a direct preference just by being custom.
Trusted table updates/replacement imports retain the local path policy as they
retain other local settings. Settings are startup snapshots, as in MIOPUNCH-38.

The selected RTT metric, five-millisecond hysteresis, one-second default and
250–30,000 millisecond accepted interval are the reversible measured initial
policy. They are not a throughput, Internet success-rate or mobile-power SLA.
There is no new congestion controller: TCP continues to use the existing BBRv3
configuration. More extensive per-node exploration/scheduling remains MIOPUNCH-48.

The existing iroh remote actor owns the timer. At each permitted interval it
pings only qualified paths on living connections and reevaluates their actual
statistics; it schedules from now rather than catching up in bursts. The
optional defaulted `PathSelector::reevaluation_interval` preserves existing
custom selectors. Product startup installs this policy via
`Builder::default_path_selector`, which preserves a caller's explicitly supplied
advanced selector. Event-driven selection still handles path establishment,
abandonment and candidate transitions; there is no parallel product path loop.

Path scores and selected status are computed separately for each Connection.
The context filters forbidden/retired custom routes and foreign physical UDP
bindings before scoring. The peer bootstrap hint comes only from qualified data
connections. Restricted exchanges retain their own paths instead of borrowing a
peer's business or signaling path. Noq's packet gates remain installed before
normal release, so queued business and remote reopen races cannot make a
signaling-only carrier a business path. An admission-gated native sole custom contact uses its own route for Initial
packets rather than another control connection's cache. Existing explicit
control-only routes retain their old rule. Ungated raw consumers retain their
historical mapped bootstrap and graceful-drain behavior.

`Endpoint::open_data_path` now also validates UDP on an admitted living
Connection. `Endpoint::open_data_transport_path` accepts an explicit UDP, native
relay or qualified custom data candidate on an ordinary admitted connection.
It does not grant carrier permission and does not reopen a closed Connection.
The native bounded coordinator retries only noq's actual path-ID-unavailable result;
public path observation, not the open request alone, establishes usability.
The previous test-only custom-path hook delegates to this same production gate.

## A new carrier while the original business connection still lives

`NativeNetwork::recover_connection_path` is an explicit connection-maintenance
SDK operation. Its contact is the same authenticated target via a currently
permitted route. `serve_connection_path_recovery` is the corresponding router
handler; configured endpoints register `CONNECTION_RECOVERY_ALPN` by default.
A dispatcher handles that ALPN just as it handles the existing native recovery
protocols. This ticket adds no new background member-selection scheduler.

A fresh restricted control exchange uses the existing admission gate and
connection-bound SCRAM proof. Both identities must still be current members.
An opaque 32-byte binding from the original business Connection's standard TLS
exporter travels only inside the protected control stream. The responder must
match it to an actual living Member session for that same peer. This is a
session reference, not an alternative admission proof, identity or crypto
handshake; an unknown or terminated binding yields no candidate offer.

Each side offers at most eight current endpoint data candidates in messages of
at most 4,096 JSON bytes. The private request contains the 32-byte original TLS
binding and the requester's `EndpointAddr`; the response contains the responder's
`EndpointAddr`. Both exclude signaling and locally disabled data carriers,
validate the offer's identity against the authenticated peer and apply their own
resource snapshot before opening a path. The original noq dialing side performs
path opening, regardless of which endpoint initiated the fresh control exchange;
the other original endpoint observes actual path establishment. Both recheck
current membership throughout validation.
Candidate addresses remain hints: the original QUIC path validation and packet
gates determine success. Candidates are attempted once; only unavailable QUIC
path identifiers get a short retry. No management record or application bytes
travel in this exchange. Exporter bindings and proof/secret material are not
logged or copied into shared records.

The attempt has a three-second bound and races the original connection's actual
close future. Its owned control handle closes on completion, failure or
cancellation. The serving handler keeps the proved exchange alive until that
close, also within its own bound, avoiding loss of an unacknowledged offer.
A successful result names the unchanged original stable ID and a real validated
path. As upstream documents, `stable_id` is local and stable for a handle's
lifetime, not a persistent globally unique session name. The TLS binding refers
to the actual original session rather than treating that numeric ID as a wire
credential. A failure reports `Retryable` only while that original still lives;
`Terminal` names its terminated ID. A live control connection never overrides
this result. Later ordinary connect returns a new Connection and streams, with a distinct
stable ID while the terminated handle is retained; the old application stream
remains failed.

## Qualified matrix and failures

The runnable matrix checks both endpoints' original stable IDs, continued use
of the exact same stream and exact received content:

| Native Linux controlled case | Qualification |
| --- | --- |
| TCP to UDP after both validate | Same Connection and stream survive |
| TCP to better permitted native relay, then back to an existing TCP backup | Same Connection and stream survive |
| UDP to explicitly preferred native relay, then back to UDP backup | Same Connection and stream survive |
| Two living TCP routes with changed physical delay, unchanged path IDs | Budgeted reevaluation selects the better route |
| Faster relay with `PreferDirect` | Explicit direct preference wins |
| Disabled relay with `PreferRelay` and an explicit path request | No origin access and no relay path |
| No prebuilt backup; original TCP proxy physically removed; new MQTT/SCRAM exchange coordinates a new TCP carrier, initiated separately by each original endpoint role | New carrier on the living original business Connection |
| Unknown original TLS binding on a proved control connection | No offer; the real business Connection remains usable |
| Original business Connection terminated while control remains alive | Terminal repair result; only a distinct later Connection |
| Only business carrier physically removed with 65,536 queued bytes, separate control still live | Original stream fails within actual lifetime; no signal fallback |

The initial real first check failed because the UDP API rejected an ordinary
admitted Connection. The controlled quality RED retained both paths with a
better actual RTT but did not switch. The isolation RED showed a live MQTT
control path redirecting an explicitly targeted TCP business attempt. Each has
a real GREEN. Stale admission route caches in the first quality fixture,
missing new APIs, a Rust borrow/import error, and an early-close control-handler
failure are classified separately. The accepting-side RED found noq's real
client-only path-opening constraint; protected two-sided candidate exchange now
delegates that work to the original dialing peer. The affected legacy raw MQTT
CLI shutdown RED was narrowed to overly broad custom Initial pinning: an exact
pre-47 runtime comparison passed, and the native-gated compatibility fix passed
three narrow reruns without changing the original three-second test window.
These records are indexed in the [evidence index](../evidence/mio47-path-maintenance/README.md).

The scope is IPv4 loopback Linux, independent persisted identities and SQLite,
real TLS/SCRAM, UDP, native TCP and a controlled native HTTP relay plus owned
Mosquitto. New TCP recovery uses an explicit reachable candidate, not a new
NAT traversal guarantee. A selected-path close demonstrates removal with an
existing backup; the only-carrier proxy removal demonstrates actual physical
loss and final failure. Broader unknown-member or arbitrary NAT compositions,
IRC-based repair, ordinary-node data relaying, native MQTT-first relay-only
upgrade, public facility availability and other-platform runtime are not claimed.
No product UI, service ACL, TUN, KCP, runtime plugin system or application-state
recovery protocol was added. Browser evidence is compilation only.

Run `rtk proxy python -B scripts/check_path_maintenance.py --output NEW_DIR`
for the ordered acceptance matrix, `--case NAME` for one case, or `--suite` for
acceptance then affected short regression/type/format/WASM checks. The runner
verifies each named case exists and exactly one test executes, retains raw logs
and command exits, and creates/removes only its owned brokers and anonymous
volumes. Already verified unchanged slow NAT matrices are skipped under the
user's explicit policy. The evidence links identify actual final-source checks.
