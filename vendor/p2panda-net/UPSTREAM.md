# p2panda-net source and local delta

Source: p2panda v0.7.1, commit `083b48215964e92a564f3c83a1c607b00e94aa64`, crate `p2panda-net/`. License: MIT OR Apache-2.0; see the repository's `LICENSES/` directory. The verified source archive is described in `docs/notes/2026-09-29-integration-browser-ticket-source-analysis.md`.

The endpoint builder hook in `src/iroh_endpoint/builder.rs` is reapplied on every bind through the existing actor arguments. Ordinary `connect` and `connect_with_config` preserve upstream's immediate `TransportInfoMissing` error when the address book has no iroh transport entry. The added `connect_resolving` and `connect_resolving_with_config` methods explicitly allow iroh's configured lookups to resolve that identity instead. Store errors still fail before lookup. Both choices share the endpoint and connection actors, which retain accept, report, and shutdown ownership. `src/iroh_endpoint/tests.rs` was adjusted for the argument tuple.

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

## MIOPUNCH-100 resolution defaults and backoff bounds

Both ordinary and restricted wrappers expose the explicit resolving methods.
Installing a builder hook does not enable resolving. The root wrapped probe
opts in and awaits endpoint close on completion or a failed send. The existing
connection actor still excludes local pre-connect and post-handshake rejection
from failed-address reports; real lookup/transport failures retain their own
errors and reporting rules, including the existing local-connectivity guard.

The added public discovery backoff configuration uses exact whole-millisecond
increment bounds in `[min, max)`, limited to `u64::MAX` milliseconds. It rejects
fractional, zero, reversed and unrepresentable increment bounds before sampling.
Initial delay and ceiling keep full `Duration` precision. Saturating addition
retains the immediate ceiling clamp even near `Duration::MAX`. The original
default ranges and reset policy remain in place.

The frozen ordinary consumer was built and run against exact upstream p2panda
source before this change, then run unchanged against the fork's failing and
restored defaults. Additional public tests exercise real resolving streams,
store failure, lookup failure and rejection-versus-reachability reporting.
See `docs/notes/2026-10-19-p2panda-resolution-compatibility.md`.
