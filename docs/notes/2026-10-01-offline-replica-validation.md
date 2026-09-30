# Durable offline management replicas

MIOPUNCH-31 implements the third-party offline-copy behavior in [the parent
specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
The fixed implementation baseline is `84f4bf933a6fc39044508e799fc00bab9e0014ca`.
The authorized public seams are `NativeNetwork`, its verified management-history
queries, actual admitted iroh `Connection` streams, and node reopen after process
failure. These are the parent specification's approved node/storage/connection
testing boundaries, rather than a new set of private actor tests.

## Selected integration

`src/offline_replica.rs` connects the existing selected management consumer to
real p2panda-sync 0.7.1 `LogSync::run` and `OperationReceived` events on the
authenticated original iroh connection. It uses a one-way source snapshot of
locally applied, authorized management records. The snapshot implements the
read-only `LogStore` methods needed by LogSync and advertises original authors'
logs, including third-party records. Its frozen manifest pins the network root,
canonical Create hash and exact exchange record hashes. It never loads private
admission-delivery tables, node/root private keys or session state.

The caller registers `REPLICA_ALPN` as `ProtocolScope::Member` and advertises it
on the node's endpoint. The existing gate requires current membership, the
current admission credential, and an allowed data path on that connection.
`serve_replica` verifies the gate before emitting the manifest and checks again
after reading the request. `sync_replica` checks it before starting and before
consuming each operation. Signaling-only carriers remain excluded by the
existing member/data-path gate. Resource configuration and synchronization
scheduling remain the established caller/connection responsibilities.

Each received event is checked against the source scope, then passed through
complete-body/signature/network validation, the durable `ManagementStore`
ingest transaction, causal readiness and `ManagementAuth`'s existing product
authority/StrongRemove rules. The authorization gate receives the resulting
view. Valid signed-extension-only actions use this same consumer even when
their body is absent. A required missing body rejects the exchange before a
durable record/hash is written; another exchange can retransmit the complete
record. Complete records awaiting dependencies remain durable but unservable.
The selected causal queue uses those durable rows without destructive dequeue;
the applied-order marker and Groups view still share one transaction, so a
process exit after selection cannot lose application.

The public `ReplicaOutcome` separately reports the authenticated source, its
frozen scope, transport Done, complete saved record hashes and locally applied
record hashes. `complete()` requires all three facts for that scope. Missing
dependencies or quarantined unauthorized actions leave it false. A subsequent
exchange or node reopen replays committed work; already applied records do not
issue another action. No pruning is enabled.

The receiver deliberately starts LogSync with empty advertised heights and
retransmits this bounded snapshot on retries. This avoids treating pending or
incomplete records as a completed log prefix and keeps the first recovery path
simple. The current limits match the existing history exchange's 4,096 records
and 16 MiB, with a bounded 20-second exchange. This is a bounded implementation
limit, not an approved network-scale or performance target. Paging/delta
optimization and broader control-record types are not delivered here.

## Real failure found and fixed

The first public-entry RED was compilation failure for the missing replica API.
After that entry was implemented, the initial three-node check passed: C
published a body-bearing signed Add, B saved it, C's endpoint and store closed,
B reopened, and A obtained the exact original C header, body and dependency
solely from B. The resulting member had Read access, and a repeated exchange
did not increase the applied count.

The prioritized child-process check initially failed **before** its requested
checkpoint: provider LogSync returned successfully and its last Connection
handle dropped, while the receiver still needed incoming data. The actual
receiver error was `ConnectionLost(ApplicationClosed(...))`. Both RED logs
remain in the evidence directory. A separate post-LogSync receiver receipt now
keeps the provider alive through durable event consumption and records saved
versus view applied separately. Transport Done alone is not that receipt.

The same check then reached a genuine process exit 81 after causal selection,
before view commit. B reopened and recovered the four-record view; after C went
offline, A fetched the original body and dependencies solely from B. Repeated
delivery retained four records. The fault provider's `Err(Transport)` is the
expected interrupted exchange, not a normal successful synchronization.

All five actual process exits subsequently passed: before/after record commit,
after causal selection, and before/after Groups-view commit. Before record
commit, B did not claim the uncommitted record and had to retry from C before C
went offline. The other checkpoints recovered from B's own committed record.
Every recovered case continued serving the complete offline copy to A.

Separate real LogSync checks passed required-body rejection followed by
restart/complete retry; application of a bodyless signed management action;
missing dependency with `transport_done=true`, saved record and empty applied
set; and later out-of-order, duplicated delivery with the dependency, producing
the correct view once. An ordinary member's signed unauthorized Add was
quarantined and excluded from B's served scope. Real node private key, admission
material, root private backup and TLS-exporter bytes were absent from shared
history; A's actual fetch from B excluded the unauthorized record.

## Verification and limits

Reproduction commands, raw logs, the final source-hash receipt, and the two
independent review reports are under
[`docs/evidence/mio31-offline-replica/`](../evidence/mio31-offline-replica/README.md).
The runner uses an owned broker with a dynamically allocated loopback port and
removes only that container. It checks source hashes through the full suite,
WASM compilation, both default and fault-build typechecking, and formatting.

The final generation completed with **268 passed, 0 failed, 4 existing ignored**.
Its five commands each exited 0; actual MQTT ciphertext capture observed 106
publishes. The seven implementation/test/dependency hashes stayed unchanged
through all checks, and the exact owned broker container was removed. The full
suite's twelve replica tests include all five actual process-exit boundaries.
Final independent Standards review found zero required findings and one optional
fixture-duplication maintenance suggestion. Spec review found zero required or
optional findings. Their reports are linked from the evidence README; neither
review repeated successful tests or altered source.

Evidence uses native Linux loopback, real SQLite/Ed25519/iroh TLS/QUIC and actual
LogSync events. Faults are actual process exits at storage/application
boundaries; physical power cuts were not tested. Browser WASM checks establish
compilation compatibility only. No new Windows/Android/browser replica runtime
or NAT probability claim follows from this work. Formal UI, service ACL/TUN,
secret rotation, precodes, migration, arbitrary business replication and
automatic replica scheduling remain outside this ticket's delivery.
