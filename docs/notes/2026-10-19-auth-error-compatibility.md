# Original auth errors and independent policy checks

This implements
[MIOPUNCH-96](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d70bb533-2c01-4f8a-9663-dd56b805ad0f)
under M91 and ADR 0012, on baseline `a6cb8adb`. It changes error contracts and
their application classification; management records, authorization policy,
storage format and synchronization wire messages keep their existing meaning.

## Library and application contracts

`GroupCrdtError` is again the original six-variant exhaustive enum, with its
declaration byte-for-byte equal to the fixed official 0.7.1 source. Ordinary
`GroupCrdt::process` retains its original result type. `process_checked` returns
the independent `CheckedGroupCrdtError`: `Group` preserves the original error,
and `PolicyRejected` identifies application-policy rejection.

Both entry points use the same private processing and validation algorithm.
Its callback returns a result in the caller's error type. Ordinary processing
supplies success directly; it needs no impossible checked-error branch, panic
or second algorithm. Checked processing runs its policy against the same
reconstructed causal predecessor as before, before graph insertion. Resolver,
group action, concurrency and replay rules remain shared.

The root `AuthError::Group` still carries the original group error. An explicit
conversion maps checked policy rejection to `AuthError::PolicyRejected`.
SQLite processing durably quarantines both policy and substantive group
rejections, while retaining the existing handling for missing state. Browser
replica processing already waits only for its explicit missing-history/log
cases; the distinct policy error follows its rejection branch.

## Verification

Before production edits, one fixed consumer compiled and ran against official
auth sources, exercising the old `process` function type, a real Create and a
duplicate-operation error through an exhaustive six-variant match. Identical
consumer bytes failed against the fork with `E0004`, then passed after repair.
The permanent consumer in `tests/auth_consumers/original.rs` contains those
same qualified bytes, registered by `tests/auth_compatibility.rs`.

The official fixture preserves every Rust source byte. Its standalone manifest
only supplies inherited workspace metadata and dependencies from the pinned
repository graph; the original upstream snapshot has no Cargo lockfile.
Selectors retain complete locked foreign dependency rows without resolving
again. The root Cargo lockfile is unchanged from M95.

The affected tests pass 21 selected entries: three compatibility tests, seven
management-auth tests, seven management-store tests and four real admitted
replica tests. One store entry is the existing subprocess worker, also invoked
by its process-restart parent test. Coverage includes checked-vs-group error
classification, causal predecessor reconstruction, refusal before insertion,
concurrent strong removal, missing dependency backfill, durable quarantine,
valid later progress and authorization after SQLite reopen/replay.

Both current Native and RTC-enabled WASM SDKs are rebuilt. Owned-loopback SDK
checks cover the real waiting request, one approval, durable authorization and
admission-secret delivery, persistent identities reopened and member PING/PONG;
restricted resource/cancellation/deadline/local-departure negatives; and Browser
incoming manual approval, precode/retry, replica and original-member RTC
improvement. The peer's still-valid authorization view before it learns a
departure remains separate from the departing Browser's refusal to deliver.
SDK/WebSocket closure is observed before context disposal; Native and relay
exit naturally, owned containers disappear, then generated private keys and
NSS databases are removed.

Builds use the shared target, committed profiles, fixed locks and pre-Cargo
source/tool/artifact guards. The final source digest is
`63c96589d15b52669a2c260e4e274ca03a24eb4fa6635eb2dc19554660323c57`.
One initial build-driver preparation rejected an explicit message-format
argument before Cargo; it is retained as a failure, not a successful build.

Evidence and separate independent review axes are in
`docs/evidence/auth-error-compatibility-2026-10-19/`. Raw private SDK identities,
stores and SDP remain local; published SDK results use a scalar/hash allowlist.
To maintain the compile free-space floor, 14 obsolete intermediate M93 test
executables are stored losslessly compressed with original hashes and restore
paths. No SDK pin, final test, official baseline, raw log or input is removed.
The unsuccessful cache/dedup reviews changed no files. All 249 pre-existing WIP
paths retain their bytes and remain outside this delivery.

These checks qualify this error-contract change and its affected application
flows; they do not complete the remaining M91 compatibility groups or replace
the independent public/physical-device matrix.
