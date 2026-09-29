# p2panda-auth source and local delta

This crate is p2panda-auth 0.7.1 from p2panda commit
`083b48215964e92a564f3c83a1c607b00e94aa64`. The source copy came from
the verified upstream snapshot at `/tmp/p2panda-0.7.1`; MIT and Apache-2.0
license texts are retained in `LICENSES/`.

The only upstream Rust change is in `src/group/crdt/mod.rs`:

- `process_checked` accepts a policy callback at the causal predecessor state
  reconstructed by upstream `validate`, before the operation enters the graph.
  Its rejection has a distinct `PolicyRejected` error. Existing `process` keeps
  its previous behavior by passing an always-true callback.
- `GroupCrdtState` and its inner state derive `Clone` outside test builds so an
  application can attempt processing transactionally and retain its old view
  when a policy or CRDT check fails.

StrongRemove's resolver, graph rules, state transitions, and existing tests are
unchanged. The product mapping and verification entry point live in
`src/management_auth.rs`; see the MIOPUNCH-23 validation note.
