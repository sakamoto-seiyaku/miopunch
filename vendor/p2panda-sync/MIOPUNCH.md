# Local p2panda-sync 0.7.1 adaptation

Copied from the locked crates.io package, checksum
`ab2dfec4540ec2021949ae1e90788948528f51d76e50827895d821ffda76272e`.
The sole upstream code/config change is `default-features = false` on the
normal `p2panda-store` dependency in the normalized `Cargo.toml`.

LogSync only needs the storage traits. Enabling the store's default SQLite
feature pulled SQLx/Tokio native networking into the WASM build. Native users
still enable their own storage features. Protocol and scheduler Rust sources
are unchanged. `Cargo.toml.orig` is retained as upstream provenance.
