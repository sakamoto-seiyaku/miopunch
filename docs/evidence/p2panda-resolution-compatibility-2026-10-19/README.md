# p2panda resolution compatibility

MIOPUNCH-100 restores the ordinary missing-transport contract and makes identity
resolution an explicit call. Implementation: `fea33a646a60f67218d50892f773ac9c81d405f9`,
against `802742264d0f26ba96ec8af41b35f5a0ab3b7829`.

See the [implementation note](../../notes/2026-10-19-p2panda-resolution-compatibility.md),
[originating task](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d0afed8e-09b3-495a-9058-296fb5c217a7),
[validation receipts](validation.json), [selected-input continuity](selected-input-continuity.json)
and [independent reviews](review.md).

## Ordinary consumer and explicit resolution

The [frozen consumer](../../../tests/p2panda_consumers/ordinary_missing_transport.rs)
was qualified before the first production edit. Its bytes are identical for
these three actual runs in the same pinned Docker image with `--network none`:

| Library source | Result with an empty address book |
| --- | --- |
| Official p2panda v0.7.1 | Existing `TransportInfoMissing` error in about 30ms |
| Fork before M100 | Timed out after 751ms |
| Final fork | Existing `TransportInfoMissing` error in about 30ms |

The original-source baseline uses official commit
`083b48215964e92a564f3c83a1c607b00e94aa64` with the existing pinned fork dependency
graph, including iroh 1.2.0. The temporary manifest flattens workspace inheritance
and selects the unchanged library source under a local package alias. It does
not certify a wholly upstream dependency stack. Complete foreign lock rows,
versions, sources and checksums were retained; failed preparation attempts
aborted under `--locked` and were never used as compatibility proof.

The ordinary methods return before lookup when a node or its iroh transport
information is missing. Both wrappers add `connect_resolving` and
`connect_resolving_with_config`, sharing the existing actors and error types.
The root wrapped probe and restricted wrapper fixtures explicitly select them.

## Actual checks

- A pre-fix public regression observed one unexpected configured lookup. The
  final ordinary methods, with and without transport configuration, make zero
  lookup calls for absent and known nodes without transport information.
- Both resolving APIs obtain addresses from the configured lookup and exchange
  real PING/PONG streams using fresh endpoints. Store failures return locally
  with zero lookup calls; an owned failing lookup returns the existing address
  error rather than a local-rejection message.
- Local rejection before connect and after TLS preserves address-book health.
  A genuine unsupported-ALPN handshake failure records stale health. That
  comparison runs with `11.0.0.1/32` assigned only inside an isolated container,
  activating the existing global-connectivity reporting guard without a default
  route. Plain loopback cannot establish this reporting distinction.
- Backoff regressions reproduce acceptance of `1ns..2ns` and duration-addition
  overflow. Final checks cover exact whole-millisecond half-open increment
  bounds, rejection of fractional/reversed/unrepresentable bounds, full-precision
  initial/ceiling values, immediate clamp, `Duration::MAX`, the largest permitted
  millisecond bound, and unchanged default random increments with a 30s ceiling.
- Eight focused tests and two affected restricted-wrapper/admission tests pass.
  Real restricted delivery and revocation remain in the existing pipeline.
- The actual wrapped CLI passes loopback, wildcard binding, and wrong-identity
  rejection followed by a valid exchange. Every server exits naturally; UDP
  ports can be rebound and the temporary identity directory is removed.
- Root `cargo check --locked --offline --features rtc-probe --tests` passes.
  Independent Standards and Spec reviews report zero findings.

The current Native SDK and the verified Browser SDK complete restricted
admission, one actual approval, durable receipt of both materials, identity
reopen on both sides, and member PONG through an owned relay. The request returns
in about 722ms. Browser SDK reuse is backed by all actual selected input and
artifact hashes: its only selected p2panda-net source is unchanged `codec.rs`.
Native and Browser actors, sockets, browser context/process, origin, relay and
owned containers are reclaimed; generated keys and private NSS data are removed.

All builds use the shared repository `target/`, committed profiles and guarded
source/lock/artifact/tool snapshots. Twenty-five prior frozen consumers remain
unchanged. The M93 hook fixture's old implicit-fallback assumption is preserved
as historical evidence, with its bytes retained; it is not presented as an
upstream p2panda consumer or a current-default success. The 249 pre-existing
working-tree files retain their original hashes and dirty states.

## Evidence and limits

`raw-verification.tar.gz` contains controllers, manifests/locks, original
p2panda-net source, compiler records, captured dependency files, relevant source
copies, runtime outputs, diagnostics and reviews. Every member was extracted
and checked against [archive-manifest.json](archive-manifest.json). Private SDK
receipts, invitations, actor/store logs, generated keys and NSS data are excluded;
the public SDK summary uses scalar facts and hashes. Immutable executables/WASM
and lossless intermediate fixture archives remain local with their hash and
restore receipts.

Six final or independently frozen build graphs retain all actual selected input
and immutable artifact hashes. These scoped checks do not replace public-network,
physical Android or the full transport matrix. Resolving retains the existing
connection actor's cancellation behavior; the call does not add an actor
cancellation guarantee. Default backoff reset code is unchanged, and no real-time
reset cycle is claimed here.
