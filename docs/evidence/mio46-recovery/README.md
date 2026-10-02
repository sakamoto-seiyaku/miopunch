# MIOPUNCH-46 evidence

The implementation and supported profile are documented in
[the recovery note](../../notes/2026-10-02-unknown-member-recovery.md).
Rerunnable entry: `rtk proxy python scripts/check_network_recovery.py --output <new-directory> --suite`.
The wrapper creates two owned real brokers, sets their actual addresses, runs the
first check then priority failure before other independent conditions, records exit
codes/stdout/timing and removes its containers. A selector runs one named check.
New runs need a new output directory. Ordinary cargo tests without the fixture
variable leave these external-service checks inert; the wrapper always enables them.

## Actual RED and diagnosis

- `first-red/`: compiled node startup returned Waiting instead of discovering and
  recovering from unknown C. This is the first behavioral RED.
- `first-green/`: a test used an unavailable Endpoint ALPN getter. Compile failure,
  not behavioral evidence; retained honestly.
- `first-green-run/`: real history exchange reached the provider but failed when
  the receiver immediately closed before its final receipt arrived.
- `first-ack-green/`: adding acknowledgement alone still left the provider dropping
  its handle before delivery. Recovery returned Waiting.
- `provider-retention/`: first actual GREEN including normal member business after
  the acknowledged exchange and bounded provider handle retention. Its temporary
  tagged observations show complete source history, authorized identities and the
  original connection's remaining IP path. Debug instrumentation was removed.
- `priority-failure/`: real Bootstrap/no-history boundary already passed; the test's
  subsequent Member attempt was suppressed by contact backoff while its fixture
  wrongly waited for a network accept. This fixture timeout is not a product RED.
- `priority-confirmed/`: independent real Member dial confirmed both sides reject
  the fresh identity despite its old-secret Bootstrap proof; no C history saved.
- `dependency-red/`: real signed LogSync input missing a causal dependency was
  saved but only a generic Waiting result was returned, losing source progress.
- `dependency-green/`: WaitingForDependencies retains saved/applied scope; a fresh
  exchange supplies the missing record and applies the pending dependent record.
- `entry-duplicates/`: early combined exploration of unavailable entry and duplicate
  hints passed. The final ordered run separates those two conditions.
- `false-candidate/`: a real advertised endpoint without the network secret failed
  Bootstrap; a later genuine C restored the same returning endpoint.

## Ordered acceptance and short regressions

`ordered/results.json` is the authoritative twelve-condition ordered acceptance
receipt: unknown C with B offline and independent public-field capture; old secret
with fresh identity; missing dependency and retry; no candidates and new contact;
one failed entry; duplicate hints; false candidate without secret; Lookup disable;
missing admission material; wrong network; wrong saved generation; known revoked
candidate. All twelve passed. Its later resources check failed because an empty
recovery MemoryLookup was installed even without qualified discovery resources;
that actual affected regression is retained (25 of 26 resource tests passed).

`continued-short/results.json` continues after restricting lookup installation to
qualified resources. It contains the affected resource, native/gate/proof,
MQTT-purpose/legacy discovery, formal MQTT, manual admission, offline-replica,
TCP, short budget/scope, library, typecheck, browser compile and format results.
Read that receipt for final verdicts rather than treating a directory name as success.
The existing slow NAT matrices were not rerun: no NAT scheduler, punching,
observation, mapping or congestion-control implementation changed.

The test fixture uses real cryptography, private per-node stores, actual sockets
and actual broker messages. Missing-dependency injection uses the existing explicit
replica-test-hooks build, and sends original valid signed records over the same
LogSync/data route. Test private files are temporary and not committed. Reported
legacy crash-worker counts include inert subprocess entry checks; these are not
additional physical recovery scenarios.

`continued-short/results.json` finished with all checks passing and both owned
containers removed (exit zero). The short test logs report 148 passing tests,
including inert worker entries; all-targets with current hooks, browser WASM
compile, root and fork formatting also passed. `reopened-startup/results.json`
adds the current-source first check with actual A store reopen; C's verified
membership remains saved. No slow NAT matrix was run.
