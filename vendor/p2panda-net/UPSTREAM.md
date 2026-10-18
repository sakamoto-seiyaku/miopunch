# p2panda-net source and local delta

Source: p2panda v0.7.1, commit `083b48215964e92a564f3c83a1c607b00e94aa64`, crate `p2panda-net/`. License: MIT OR Apache-2.0; see the repository's `LICENSES/` directory. The verified source archive is described in `docs/notes/2026-09-29-integration-browser-ticket-source-analysis.md`.

The runtime local delta is limited to an endpoint builder hook in `src/iroh_endpoint/builder.rs`, passing that reusable hook through `src/iroh_endpoint/actors/endpoint.rs`, and falling through to iroh's identity-only lookup when the p2panda address book has no iroh transport entry. The endpoint actor retains accept, report, and shutdown ownership. `src/iroh_endpoint/tests.rs` was adjusted for the new argument tuple.

The test-only `src/test_utils.rs` listener port range is `20000..30000` instead of upstream's `49152..65535`. During [Pixel6a validation](../../docs/notes/2026-09-29-android-pixel6a-validation.md), upstream's range overlapped this Linux host's ephemeral-client range and intermittently made unrelated tests fail with `Address already in use (os error 98)` during endpoint bind. This does not change production endpoint binding.

This source is vendored so the small adapter change and the resolved dependency graph build reproducibly without a private Git fork. Other p2panda crates are pinned registry dependencies in `Cargo.lock`.

## MIOPUNCH-93 restricted endpoint adapter

`Builder::spawn_restricted` explicitly constructs a separate restricted wrapper.
Its endpoint getter returns `iroh::RestrictedEndpoint`; it has no ordinary
endpoint conversion. Private ordinary/restricted ownership variants reuse the
existing endpoint and connection actors, address-book lookup, reports and
shutdown. The restricted actor delivers a business connection only after the
new admission pipeline succeeds. Ordinary `spawn`, endpoint return types and
hook handling retain their original contracts.

An immutable ordinary public consumer was built and run before this change and
is compiled unchanged afterward. The typed wrapper additionally exercises real
member streams and revocation. See
`docs/evidence/native-restricted-admission-2026-10-19/README.md`.
