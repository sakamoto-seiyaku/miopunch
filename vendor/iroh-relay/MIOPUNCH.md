# miopunch packaging change

Source: crates.io `iroh-relay` 1.2.0, matching the existing locked dependency.
Package checksum: `beb2294a9749d6a25fd7cd8bcf0fccd932f20716d4f967d85d3f23c7135ae6a2`.
All upstream Rust sources are unchanged.

The normalized manifest builds only the Rust library (`lib`), omitting the
unused `cdylib` artifact. Building the upstream dual lib/cdylib package for
x86_64-pc-windows-gnu with Rust 1.98.1 exceeds the Windows PE export ordinal
limit (even client-only: 66063). miopunch links the Rust library and exposes
no relay C ABI; the unused DLL is not needed. This does not change the relay
protocol, Rust API, TLS, or the endpoint connection interface.
