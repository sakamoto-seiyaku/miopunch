# Local p2panda-sync 0.7.1 adaptation

Copied from the locked crates.io package, checksum
`ab2dfec4540ec2021949ae1e90788948528f51d76e50827895d821ffda76272e`.
`Cargo.toml.orig` retains the original upstream manifest. Protocol, scheduler,
and all other Rust sources remain byte-identical to the published crate.

The normalized manifest has two dependency adaptations:

- The common `p2panda-store` dependency disables defaults. On targets other
  than `wasm32-unknown-unknown`, a target dependency restores upstream store
  defaults (SQLite and macros), including when sync itself is used with
  `default-features = false`. The Browser SDK uses only the store traits.
- `futures-channel/sink` is explicit because sync uses its channel senders as
  sinks. Upstream native SQLite enabled this transitively through SQLx; a
  traits-only consumer must not depend on that incidental feature union.

Cargo's supported target predicate is
`not(all(target_arch = "wasm32", target_os = "unknown"))`. WASI and other
non-Browser targets retain upstream defaults; this does not qualify SQLite
or the SDK for WASI. No build-script cfg or feature subtraction is used.

The upstream `test_utils` feature and own dev dependencies remain SQLite
fixtures. Explicit Browser `test_utils` and sync's own Browser dev tests are
therefore outside the supported traits-only build. They are distinct from
the actual Browser SDK's normal and dev graphs, which must stay SQLite-free.
Native workspace/dev feature unions may include SQLite and test utilities.

Fixed public consumers in `tests/sync_consumers/` cover transitive SQLite,
macros, cursor persistence and reopen, and Browser trait imports. Their
standalone dependency graphs are checked separately from the root workspace,
whose direct SQLite dependency could otherwise hide a regression. MIOPUNCH-97
records original-consumer failures, feature graphs and complete SDK replica
verification, save, apply and replay evidence.
