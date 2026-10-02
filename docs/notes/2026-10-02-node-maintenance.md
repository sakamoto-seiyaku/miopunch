# Budgeted native startup and partial connectivity

[MIOPUNCH-49](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ad7f5e7e-8627-4ad7-bdc9-90959346ab0e)
adds sustained node maintenance to the existing
[unknown-member recovery](2026-10-02-unknown-member-recovery.md),
[source-scoped LogSync](2026-10-01-offline-replica-validation.md), and
[same-Connection path policy](2026-10-02-connection-path-maintenance.md).
The current ticket and parent specification define the scope; the earlier note's
historical reference to ticket 48 as a scheduler is superseded by the actual tickets.

## Recorded integration choices

`NativeNetwork::maintain_network` consumes a loaded durable node, binds its saved
identity and resource snapshot, serves admitted replica exchanges, and runs one
node-level contact/sync coordinator without business requests. The returned
`MaintainedNetwork` owns cancellation and provides the same Endpoint, allowed
address, node view, a watchable maintenance result, new-contact/network events,
and business dialing. `maintain_network_with` adds application Router handlers;
their ALPN admission scope is registered on the node's gate before startup.
Closing or dropping the handle requests shutdown rather than leaving the network.
Legacy explicit `resume_network`/`recover_network` and management SDK operations
remain available; this ticket does not introduce a general management RPC layer.

The coordinator uses the installed p2panda `RandomWalker` and its breadth-first
sampling, with an ephemeral SQLite address-book projection, and reuses p2panda's
random incremental `DiscoveryBackoff`. Its backoff now clamps the ceiling and
can supply its duration to the coordinator's single timer. It deliberately does
not spawn another set of Discovery, Gossip or Sync actors: those managers own
their own timers, protocols and topic membership, whereas this network already
requires the validated admission/recovery protocol and durable consumer. Actual
history exchange remains the existing p2panda LogSync. This is a narrow integration
adapter, not another gather/punch/reconnect engine.

One `ResourceEndpoint` is shared by maintenance and business dialing. Its contact
budget has a cancellation-safe, identity-scoped in-flight guard and the existing
1/2/4/8/16/32-second failed-contact backoff. A duplicate concurrent dial reports
temporary unreachability; dropping a canceled attempt releases its guard.
An identical hint does not reset backoff. Changed hints/network events may end a
long wait while retaining the node's frequency floor. A later business dial uses
the same guard rather than forcibly clearing failure pacing.
Background recovery retains prior failure history through a successful QUIC
handshake and resets it only after verified, saved source-scope completion.
The independent review found that transport success previously erased later
protocol failure history; real repeated authenticated invalid-manifest exchanges
now increase their contact gaps rather than restarting the one-second delay.

Known permitted contacts are sampled before expanded discovery. At most the
configured contact batch runs concurrently, rather than waiting for every old
address in sequence. MQTT expansion reuses the already qualified four-origin,
sixteen-candidate discovery profile, its fresh nonces and source-bound route tags.
Candidate metadata remains untrusted. Unknown-source history still requires real
TLS/SCRAM, IP path validation/commit, signed-record verification and source-scope
completion with both identities currently authorized. Known ordinary members may
use the already permitted data carriers and Member-scoped replica protocol.
Signaling-only carriers never become ordinary synchronization or business paths.

Successful, authenticated data contact addresses are privately and atomically
saved in `maintenance.contacts.json` with fsync and existing private-file modes.
The file is limited to 64 contacts, eight paths per contact and 64 KiB. Only
currently authorized, successfully verified contacts are saved; hints alone grant
no authority. Reopening validates the file's bounds, restores permitted contacts
to the endpoint's local address lookup, and reauthenticates before history use.
Addresses remain fallible contact history, not a persistent QUIC session.

The permanent neighbor view is bounded. A full view still permits one temporary
exchange with another sampled source to improve state coverage, then closes it;
the healthy original neighbor remains. Each established neighbor periodically
fetches the newest applied serving snapshot on its existing Connection. Providers
use immutable signed-history snapshots with the same live admission gate, allowing
the durable consumer to run concurrently without exposing unverified records.
Completion remains `ReplicaOutcome` for that source and scope, never global freshness.

`MaintainedNetwork::connect` returns a Connection and a `BusinessActivity` lease;
applications retain the lease while using it. Advanced Endpoint users can also
take an explicit lease. Business priority suppresses further background contact,
lookup and sync work. An already admitted bounded LogSync exchange finishes;
canceling it merely because business began had genuinely closed the session in
the RED check. Work is scheduled from the current time after release, without
catch-up bursts. This is explicit SDK cooperation, not automatic inference of
application intent from opaque QUIC packets or a bandwidth/power SLA.

Waiting states include cause, next maintenance-cycle delay, actual attempts and
lookup rounds, observed concurrency, source progress, neighbor stable IDs and
actual QUIC PING counts. An empty permitted contact/entry set is reported explicitly.
Losing the final neighbor switches to temporary unreachability even if an earlier
source exchange completed; historical progress cannot keep a live Connected flag.
Partial connectivity, temporarily unreachable entries and missing dependencies
do not become claims that all members are offline. QUIC still determines actual
connection lifetime; new sessions do not transparently resume old application
streams. A numeric stable ID is local to a retained Connection's lifetime.

## Reversible measured initial policy

Saved `ResourceConfig.maintenance` defaults old files to `conservative()`. Users
may select `active()` or bounded local settings; there are no device-class or
permission roles. Existing resource imports/updates leave local choices intact.
Configuration changes take effect at a fresh startup, as with existing resources.

| Local policy | Exploration floor | Discovery floor | Contact batch | Permanent neighbors | Quality interval | Contact timeout |
| --- | --- | --- | --- | --- | --- | --- |
| Conservative | 5 s | 15 s | 1 | 1 | 5 s | 2 s |
| Active | 1 s | 5 s | 4 | 4 | 1 s | 2 s |

Accepted limits are 250–60,000 ms exploration, 1,000–120,000 ms discovery,
1–8 concurrent contacts/permanent neighbors, 250–30,000 ms quality and
100–30,000 ms contact timeout. Discovery failure adds a randomized increment
from the configured discovery floor to that floor plus 100 ms, capped at 120 s;
p2panda retains its existing randomized 60–180 s reset mechanism. New evidence
can reset that outer delay but cannot clear an existing same-hint contact failure.
The quality interval is the slower of saved path policy and maintenance budget.

The measured controlled failure check, with a 500 ms contact timeout, observes
`(attempts, concurrent contacts, actual UDP targets)` of `(1,1,1)` versus `(4,4,4)`
within two seconds. Thus the budget changes physical effort. On the same saved
identity and a live IPv4 data session, a 3,500 ms window observes zero additional
QUIC PINGs with the five-second quality budget and three with the one-second
budget. The quality RED caught later default-selector configuration overwriting
the budget; startup now explicitly installs the budgeted selector. No unreliable
public keepalive/QNT value is used as proof of enforcement. Existing five-ms RTT
hysteresis, eligible-path selection, TCP BBRv3 and connection-local fork timers
remain the implementation, with no product path-probing loop.

## Qualification and limits

The actual matrix uses native Linux loopback IPv4, independent identities and
SQLite, real admission, signatures, TLS/SCRAM/QUIC/LogSync, owned MQTT brokers,
controlled UDP failure receivers and public SDK/Connection observations.
It includes startup without business, continuous failure, new/duplicate clues,
restart contacts, stale known contacts followed by unknown-member expansion,
two node budgets, real 32 KiB business traffic, in-flight sync during business,
partial-link propagation with the original author offline, full neighbor views,
and resource disables. Existing relevant carrier/permission/path tests are the
short regression boundary; unchanged slow NAT matrices are skipped per the user.

The automatic unknown-member discovery profile remains MQTT plus validated IPv4
UDP, as in ticket 46. This does not claim new arbitrary NAT, IRC unknown discovery,
relay-only unknown bootstrap, browser/background runtime, other-platform runtime,
public resource availability, automatic ordinary-member data-service enablement,
multihop, service ACL, client UI, TUN, KCP or generic application recovery.
Applications needing explicit new-carrier coordination on a retained business
Connection still have ticket 47's established SDK operation; this coordinator
does not add a competing recovery loop or extend actual session lifetime.

Rerun `rtk proxy python -B scripts/check_network_maintenance.py --output NEW_DIR`
for ordered acceptance, `--case NAME` for one case, or `--suite` for acceptance
and affected short regression/type/format/WASM. `--suite --start-at NAME` continues
only the remaining checks. `--review-checks` adds the affected resource/recovery,
type and format checks after the narrow review correction. Each exact selection verifies one passing test;
owned brokers and their volumes are removed in the runner's finally block.
See [the evidence index](../evidence/mio49-maintenance/README.md) for actual
results, failures, corrective checks and independent review reports.
