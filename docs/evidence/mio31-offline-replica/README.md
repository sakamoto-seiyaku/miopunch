# MIOPUNCH-31 evidence and reproduction

Public acceptance entry: `tests/offline_replica.rs`. Runtime:
`src/offline_replica.rs`, using the existing durable management consumer.
Read the [integration and failure analysis](../../notes/2026-10-01-offline-replica-validation.md).
Baseline: `84f4bf933a6fc39044508e799fc00bab9e0014ca`.

Run the final checks in a **new** output directory. The runner creates an owned
anonymous MQTT broker using `scripts/mosquitto-test.conf`, performs a TCP
preflight, sets the real broker environment, records clocks/commands/exits and
source SHA-256, then removes its exact container ID. It never overwrites an
existing result generation.

```sh
rtk proxy python scripts/check_offline_replica.py --output /tmp/mio31-reproduction
```

For the focused native suite, including process-exit checkpoints and input
faults on real connections:

```sh
rtk proxy cargo test --features replica-test-hooks --test offline_replica -- --nocapture
```

The explicit `replica-test-hooks` feature exposes only fault observation and
wire-fixture injection. Ordinary builds have neither interface. All fixture
inputs still traverse the same admitted connection, LogSync events, signature
and completeness verifier, durable SQL transactions and selected authorization
consumer. Child workers are no-ops without their dedicated test environment.

| Evidence | Observation |
| --- | --- |
| `initial-public-api-red.md` | Exact command/exit and missing API diagnostics from the initial compilation RED. |
| `first-real-check.log` | Actual C offline, B node/store reopen, A solely from B: original author/header/body/dependency, correct authorization, duplicate exchange once. |
| `priority-causal-selection-crash.log` | Initial priority child fails before checkpoint with Transport, exit 101 rather than requested 81; no crash-recovery pass claimed. |
| `priority-crash-protocol-diagnosis.log` | Provider returns Ok, drops its last Connection; actual receiver ApplicationClosed error. |
| `priority-crash-recovered.log` | Separate durable receipt fixes lifetime; actual child exit 81 after causal selection, recovered identical view and continued offline serving. |
| `missing-body-retry.log` | Required body absent rejects before durable hash; complete retry after B reopen succeeds. |
| `no-body-action.log` | Legal signed-extension-only Add applied by the chosen LogSync consumer. |
| `missing-dependency-replay.log` | Wire Done with saved record but empty applied set is incomplete; restart/later dependency/out-of-order/duplicate input applies once. |
| `all-durability-boundaries.log` | Five real exits: before/after record commit, causal selection, before/after view commit; recovered/retried four-record view and offline serving. |
| `unauthorized-private-boundary.log` | Unauthorized signed Add not served; real private/admission/session-derived material excluded; A's actual B-only scope omits denied record. |

Fault-provider `Err(Transport)` is the expected interruption, not a successful
normal exchange. A record interrupted before storage commit must be retried
from a source; the evidence does not claim recovery of uncommitted bytes.

`final-generation1/receipt.json` identifies the exact full-suite generation and
source snapshot. Its per-command raw logs cover the real-broker workspace
suite, browser WASM, default/fault-build typechecks and formatting. Source or
review-driven changes require a new result generation; old failures and
receipts remain historical evidence.

Final generation 1: **268 passed, 0 failed, 4 existing ignored**. Full real MQTT
capture observed **106 publishes**; no broker checks were skipped. WASM, default
and fault-build typechecks, and formatting all exited 0. Implementation/test/
dependency hashes remained unchanged and the exact owned broker was removed.

Independent final reviews: [Standards](final-standards-review.md) found **0
required, 1 optional** fixture-duplication observation;
[Spec](final-spec-review.md) found **0 required, 0 optional**. Both reviewers
independently inspected the staged implementation, source hashes and lossless
logs without repeating the suite or NAT checks. Reports and this publication
update are the only additions after the reviewed 27-file snapshot; runtime,
tests and check commands remain unchanged.

The short checks use real native loopback connections and durable SQLite.
Session-derived/private byte exclusion is a concrete boundary test, not a
cryptographic security audit. Physical power loss, platform runtime expansion
and new NAT probabilities were not verified or rerun.

Raw logs are stored losslessly as `.log.gz` files because test progress output
contains trailing whitespace. `lossless-log-receipt.json` retains each original
path, size/hash, compressed size/hash and identical decompressed hash. Read a
listed log with `rtk proxy gzip -dc <path>.log.gz`; the table names above denote
the original logical log paths. The final check receipt's command clocks and
exits are unchanged.
