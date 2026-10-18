# Native sync store defaults and Browser feature boundaries

MIOPUNCH-97 implements the accepted compatibility rule in
[ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md).
The pre-change baseline is `f3b52078de257c6b7b09e851259b87c1a6d629c6`.
It changes dependency declarations, with every sync Rust source remaining
byte-identical to published p2panda-sync 0.7.1. No synchronization algorithm
or wire format changes are included.

The original sync crate enabled p2panda-store defaults transitively even when
the caller disabled sync's own defaults. The Browser adaptation had disabled
store defaults for every target, removing native SQLite and macros. A fixed
public consumer exercises `SqliteStoreBuilder`, `tx!`, a real cursor transaction,
pool closure and durable reopen. It succeeds against the original published
crate with sync defaults both on and off. Against the pre-change fork, a
separate channel-feature omission first causes ten Sink method errors. Enabling
only the missing channel sink in a diagnostic fixture then reaches the unchanged
consumer and fails on its SQLite/macros imports. Both failures are retained.

The common store dependency stays traits-only. A normal target dependency with
`cfg(not(all(target_arch = "wasm32", target_os = "unknown")))` restores the
original store defaults outside the actual Browser SDK target. Sync also declares
`futures-channel/sink` explicitly: its senders use this trait, previously enabled
indirectly by SQLite's SQLx graph. Neither change adds a public sync feature.

| Consumer graph | Store features after the change | Check |
| --- | --- | --- |
| Original native, sync defaults on/off | default, macros, sqlite | Same fixed consumer builds, transaction and durable reopen pass |
| Current native, sync defaults on/off | default, macros, sqlite | Same fixed consumer builds, transaction and durable reopen pass |
| Current native, explicit sync test_utils | default, macros, sqlite, test_utils | Graph, build, transaction and durable reopen pass |
| Standalone Browser platform consumer | none | Before: Sink errors; after: wasm32-unknown-unknown check passes |
| Actual Browser SDK normal/dev | none | Graph has no SQLx; full SDK builds and runs |
| Native workspace/dev | default, macros, sqlite, test_utils | Actual workspace graph checked |
| Sync's own native/Browser dev graph | default, macros, sqlite, test_utils | Original and fork own-dev graphs checked with the exact published dev lock |
| Explicit Browser sync test_utils | sqlite, test_utils | Upstream SQLite fixtures, outside traits-only Browser build support |
| wasm32-wasip1 normal | default, macros, sqlite | Upstream defaults retained; graph check only, no WASI build claim |

The first own-dev inspection selected a dependency subtree and did not activate
sync's own dev dependencies; it is not used as own-dev evidence. The actual
package-root checks use standalone copies of the published and fork packages
with the exact published Cargo.lock. The missing pinned dev crates were fetched
with `--locked`; neither this lock nor the repository lock was re-resolved.
The upstream own test fixtures and explicit `test_utils` remain SQLite-oriented.
They do not describe the actual Browser SDK dev graph.

The permanent consumers in `tests/sync_consumers/` are exact copies of the
before-edit fixtures. Their standalone selectors deliberately request store
without defaults, so only sync can activate SQLite/macros. Testing those imports
only in the main workspace would allow its direct SQLite dependency to mask
this regression. Frozen selector manifests, full foreign lock rows, drivers,
source hashes and raw compiler results are preserved in the evidence archive.

Seven management-store entries and four offline-replica tests pass, including
signed body persistence across a child-process restart, rejection/quarantine,
no-body actions, out-of-order and duplicate convergence, unauthorized replication,
and an offline author's history supplied by a restarted member. One store entry
is the existing subprocess worker, also invoked by its process-restart parent.
These checks use the existing normal runtime features, without adding crash-hook
coverage or claiming the full physical/public matrix.

Three current Native/Browser SDK scenarios pass on owned loopback facilities:
restricted request, actual approval and durable material completion, both node
identities reopened, and ordinary member PING/PONG; five negative release cases;
and Browser incoming manual/precode admission, member communication and active
RTC improvement. The last scenario also transfers five complete signed records,
checks exact saved/applied scope, reopens the recipient, compares its records,
pending count and member permissions, and repeats the fetch without adding a
record. The replay comparator sorts member IDs because enumeration order is
unspecified. Its initial order-sensitive assertion failure is retained; the same
unchanged SDK artifacts pass the corrected semantic check.

All SDK actors close normally, native processes exit zero, WebSockets close
before browser contexts, relay processes stop gracefully, and owned containers
are removed. Generated private keys and NSS stores are removed only after these
checks. Raw SDK logs, identities, SDP and private stores remain outside committed
evidence; published validation uses an explicit scalar/hash allowlist. The
standalone consumers remove their owned SQLite fixtures after closing pools.

Builds use the shared target cache with frozen source/lock/artifact/tool guards.
The Browser SDK reports zero rebuilt artifacts; its effective graph is unchanged.
Nine obsolete intermediate M93 test executables were losslessly compressed after
hash verification and decompression roundtrip, preserving final tests, every SDK
pin, raw logs and dependency libraries. The compile disk floor was not lowered.

Receipts and the verification archive are linked under
[the M97 evidence directory](../evidence/sync-store-feature-compatibility-2026-10-19/validation.json).
The broader M91 convergence and public/physical matrix remain separate work.
