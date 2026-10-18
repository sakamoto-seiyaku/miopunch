# p2panda-auth source and local delta

This crate is p2panda-auth 0.7.1 from p2panda commit
`083b48215964e92a564f3c83a1c607b00e94aa64`. The source copy came from
the verified upstream snapshot at `/tmp/p2panda-0.7.1`; MIT and Apache-2.0
license texts are retained in `LICENSES/`.

The upstream Rust changes are in `src/group/crdt/mod.rs`, with the new error
re-exported by `src/group/mod.rs`:

- `process_checked` accepts a policy callback at the causal predecessor state
  reconstructed by upstream `validate`, before the operation enters the graph.
  It returns an independent `CheckedGroupCrdtError`, separating the original
  group error from `PolicyRejected`. Existing `process` and the exhaustive
  `GroupCrdtError` retain their original contracts. Both entry points use one
  processing and causal-validation algorithm; ordinary processing supplies a
  successful policy callback without any checked-error conversion or panic.
- `GroupCrdtState` and its inner state derive `Clone` outside test builds so an
  application can attempt processing transactionally and retain its old view
  when a policy or CRDT check fails.

StrongRemove's resolver, graph rules, state transitions, and existing tests are
unchanged. The product mapping and verification entry point live in
`src/management_auth.rs`; see the MIOPUNCH-23 validation note.
