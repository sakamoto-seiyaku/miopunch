# MIOPUNCH-49 evidence

The actual ticket is
[budgeted startup and partial connectivity](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ad7f5e7e-8627-4ad7-bdc9-90959346ab0e).
The implementation and reversible choices are recorded in
[the node-maintenance note](../../notes/2026-10-02-node-maintenance.md).

The public native SDK, Endpoint/Connection and stream seams were confirmed in
the parent specification and standing user instructions. No legacy Go/OpenSpec
gate, speculative architecture ticket, executable freeze or duplicate source
archive was introduced. Unchanged slow NAT matrices were skipped under the
user's explicit test policy; the affected acceptance and short checks remain real.

## Authoritative final verification

[final-suite/results.json](final-suite/results.json) lists the ordered eleven
real acceptance cases, followed by affected resources, native admission,
unknown-member recovery, replica, path-maintenance, member-relay, MQTT-purpose,
TCP, library, all-targets, WASM and formatting checks. Raw logs retain exact
test counts and observed effort. The first true startup check precedes the
continuous-failure check. Runner cleanup records owned broker exits.

All eleven acceptance cases and all final checks pass. Short regression reports
256 passing entries; one pre-existing flaky gossip test remains explicitly
ignored in the workspace library log, and existing inert worker entries remain
identifiable in their raw logs. Typechecking, browser WASM and formatting pass.
Both owned brokers/volumes were removed with exit zero. No slow NAT matrix ran.

Cases include new/duplicate contact hints and restart, conservative/active
actual UDP effort, actual business and shared identity guarding, original-author
offline propagation on the same neighbor session, actual quality PING frequency,
resource disables, failed saved contact expansion, in-flight business boundaries,
and a full permanent neighbor view's temporary exploration.

## Retained failures and corrective slices

- [first-red/results.json](first-red/results.json): initial no-business startup
  genuinely failed to fetch/save the unknown member.
- `first-green`, `first-diagnosis`, `gate-diagnosis`, `path-diagnosis`: early
  implementation still rejected history because the public path snapshot retained
  the just-retired signaling path after packet-gate commit. The public IP-only
  path observation is now awaited; `first-path-green/results.json` passes.
- [priority-red/results.json](priority-red/results.json): actual discovery gaps
  remained approximately 3,005/3,004/3,003 ms rather than backing off.
  `priority-green` is separately the expected manifest/locked-file failure;
  `priority-backoff-green/results.json` verifies real p2panda backoff and recovery.
- `new-clue-api-red` is an explicitly separate missing-API RED. `new-clue-green`
  and `new-clue-diagnosis` retained the missing ordinary replica ALPN registration;
  `new-clue-member-green/results.json` verifies new contact/restart and two
  necessary attempts despite 32 duplicate hints.
- `budget-first`, `business-first`, `partial-first`, `disabled-runtime` and
  `expanded-first` are real passing slices with physical/stream/storage evidence.
- [quality-first/results.json](quality-first/results.json): both saved quality
  budgets produced the same actual probe rate. `quality-green`/`disabled-first`
  retain an unrelated new test's key API compile error. After proper selector
  installation, `quality-runtime-green/results.json` observes PING deltas 0/3.
- [in-flight-business-red/results.json](in-flight-business-red/results.json):
  beginning business canceled the already admitted real LogSync session.
  `in-flight-business-green/results.json` verifies bounded in-flight completion
  and suppression of further background work.
- [full-neighbor-red/results.json](full-neighbor-red/results.json): a full
  permanent view starved a newly reachable source with newer signed records.
  `full-neighbor-green/results.json` verifies temporary source exchange while
  retaining the old neighbor and its permanent-view limit.

Fixture credentials, keys and SQLite stores stay in temporary test directories;
only public diagnostic fields and necessary test evidence are retained. These
checks are bounded Linux qualification, not a cryptographic audit, Internet/NAT
success-rate promise, platform/power SLA or transparency promise for old business
sessions. Independent Standards and Spec reports are linked after review.
