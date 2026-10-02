# MIOPUNCH-51 browser control evidence

Reproduce with `rtk proxy python -B scripts/check_browser_control.py --output /tmp/mio51-check --checks`.
The implementation and qualified boundaries are in
[the browser control note](../../notes/2026-10-02-browser-control-replica.md).

## Result

All 19 commands in `final-suite` exited 0. Nine new browser scenarios and the
previous eleven-case browser suite passed. The eight affected native suites
reported 88 passing entries: four inert crash-worker entries and one no-broker
MQTT early return are included in that count and are not network acceptance.
Both subsequent targeted browser checks also passed, as did native/WASM builds,
workspace typechecking and formatting. Implementation commit:
`f50b648c3fd791e1c3827f522bc14325cc42223c`.

## Runs

- [final-suite/results.json](final-suite/results.json): actual commands, exits,
  durations and environment versions. [scenarios.json](final-suite/scenarios.json)
  records the nine control scenarios and the previous eleven-case browser suite.
  Each control scenario uses real HTTPS/WSS relay access and real QUIC/TLS,
  signatures, LogSync and IndexedDB. The three-node scenario checks an original
  author's nonempty body after that author exits and the browser provider reloads.
- [signing-window-check/results.json](signing-window-check/results.json): targeted
  management rerun after adding a final expiry check immediately before signing,
  matching the native boundary even if graph preparation consumes the window.
- [connection-lifetime-check/results.json](connection-lifetime-check/results.json):
  targeted revocation rerun after switching the gate's session tracking to bounded
  weak handles so tracking cannot keep a dropped connection alive.

The first incremental browser replica run failed at the absent `sync_replica`
SDK method, while existing admission/reload passed. The first three-node green
preceded the multiple-page precode failure check. Both were then included in
`final-suite` in that order. The initial transport iteration exposed missing
fixture protocol registration and receipt loss on immediate connection closure;
the final fixture registration and receipt acknowledgement are exercised by the
successful reruns. This evidence does not claim those development failures were
passing runs or that the final full suite predates the two targeted fixes.

## Failure classification

Missing bodies and an injected real IndexedDB abort intentionally make the peer
report an interrupted/rejected replica exchange. The abort can also print the
browser's TransactionInactiveError. These are expected negative scenarios: the
caller rejects, the prior view survives reload, and a subsequent complete retry
succeeds. An authorization-only precode request is intentionally interrupted;
the original provider logs the incomplete delivery before the unchanged request
completes later. Corrupt saved state is intentionally rejected on repeated load.
These messages are not counted as successful transfers.

`AUTHOR_OFFLINE exit=0` records the stopped original authors. Other owned native
and relay fixtures log `CLEANUP` exits. Private bearer invitation responses are
redacted by the harness before logging; the ordinary manual invitation contains
only the public anchor and contact. Temporary browser profiles and native stores
are removed by the fixture's own cleanup.

The native regression set is limited to the affected public management/store,
replica, precode, delivery and gate seams. It includes inert crash-worker entries
which only do work when their parent supplies environment variables, and the
existing admission-gate MQTT case returns early when no broker is configured.
Those are not new browser or MQTT network qualification. Unchanged slow NAT
matrices were excluded under the user's standing instruction. No production
public-origin/certificate deployment, other browser/OS, browser direct path or
background-residency claim is made.

Independent [Standards](standards-review.md) and [Spec](spec-review.md) reports
are complete; [aggregation](code-review.md) preserves the separate axes.
Standards: 0 documented violations and 2 optional duplication heuristics, parked
under the user's completion priority. Spec: 0 findings. Both reviewers were
GPT-6 Astra Low with priority confirmed by root app-server readback.
