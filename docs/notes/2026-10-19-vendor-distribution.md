# Vendor targets and distribution artifacts

2026-10-19, [MIOPUNCH-105](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/97aefd92-d919-4b5c-89c3-47c97db642f0).
This follows [ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md)
and the existing [shared-cache rules](../agents/build-cache.md).

The Noq manifests declared nine absent example/test/bench entry files. Eleven
original files are now restored from the fixed published 1.3.0 packages: the nine
entries plus the common example module and example README. Their bytes match the
original packages; the consumers have not been rewritten for this fork. Noq's
published standalone lock is restored with only the companion Noq-proto record
changed to local source. Noq-proto's lock is the exact published lock. No foreign
version, source, checksum or dependency row is re-resolved.

Before the first repository edit, the unchanged original `connection`,
`single_socket` and `insecure_connection` examples were compiled and run in
network-none containers: five actual connections, all examples exited normally.
After restoration, Noq's default `--all-targets --no-run` succeeds. The optional
post-quantum target remains conditional on `__rustls-post-quantum-test`; a default
all-targets command does not compile or exercise that target.

From the repository root, the standalone entry uses the sibling Noq-proto fork:

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/noq-examples -- \
  build --locked --offline --manifest-path vendor/noq/Cargo.toml --examples --jobs 2
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/noq-targets -- \
  test --locked --offline --manifest-path vendor/noq/Cargo.toml --all-targets --no-run --jobs 2
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/noq-send-buffer -- \
  test --locked --offline --manifest-path vendor/noq-proto/Cargo.toml \
  --bench send_buffer --features bench --no-run --jobs 2
```

Use a fresh receipt directory for each invocation, keeping the shared target and
committed profiles. The qualification driver additionally checks fixed source,
all declared target files, complete lock rows, tools and retained SDK artifacts
before invoking Cargo. Missing target content or changed preconditions abort
before compilation. Its exact versions are retained with the evidence.

## Package content and local fork selection

The normalized published snapshots contain `Cargo.toml.orig`. Directly running
`cargo package` against the original snapshot fails because that is a reserved
source member: Cargo creates it during packaging. In a VCS checkout the original
`.cargo_vcs_info.json` is also reserved. Both restored manifests exclude those
snapshot copies when packaging. The original manifest, VCS provenance and licenses
remain in the repository; Cargo generates its own member in the resulting archive.

The packages still have their fixed upstream names and versions. They are local
fork sources, not newly published crates.io releases. In particular, the Noq fork
uses additional Noq-proto methods. Cargo does not propagate a dependency's
`[patch.crates-io]` table to an application, and generated package locks refer to
registry dependencies. An application selecting these sources must patch both
`noq` and `noq-proto` at its own root, as the miopunch workspace already does.
A bare Noq `.crate` with registry Noq-proto is not a supported distribution.

These source-package checks cover Noq and Noq-proto. Other vendored crates remain
source snapshots selected by the root's patches; their independent registry
package builds are not qualified here. In particular, iroh and portmapper's
published standalone locks remain absent: their supported frozen entry is the
root Cargo.lock and full patch graph. Use the shared-cache runner with
`build --locked --offline -p iroh --lib` (or `-p portmapper --lib`) from this root
for those Rust-library entries. Library use in that graph is distinct from
repackaging a standalone release. Declared files in the other snapshots are
present; this ticket does not call every vendor target/platform build verified.

The source archives and their complete member hashes are checked separately
from compilation. `package --no-verify` is only package-content evidence. A
standalone package consumer must also use the co-distributed Noq-proto source
and the fixed standalone lock with its one local package record. This requirement
is retained in the distribution receipt; package creation alone is not called
runtime or dependency compatibility.

## Rust library, explicit cdylib and relay CLI

Ordinary iroh and iroh-relay manifests retain `crate-type = ["lib"]`. The prior
mixed lib/cdylib default is an explicit remaining artifact difference. It is
not restored because unversioned output can overwrite another cached graph
while Cargo reports it Fresh, and the prior Windows GNU build exceeded the PE
export ordinal limit. Rust consumers retain their library entry; a caller that
previously expected an automatic DLL/shared object must use explicit generation.

Linux cdylib generation now has a reusable entry. For example:

```sh
rtk proxy python3 scripts/build_vendor_cdylib.py prepare \
  --package iroh --output /tmp/iroh-cdylib
rtk proxy python3 scripts/build_vendor_cdylib.py run --output /tmp/iroh-cdylib
```

Use `--package iroh-relay` and a new output directory for relay. `prepare` accepts
repeated `--preserve-receipt /absolute/path/to/receipt.json` arguments to pin
existing frozen SDK inputs as preconditions. `run` checks sources, locks, stable
tool binaries and those artifact hashes before Cargo and again under the shared
cache lease. It invokes `cargo rustc --lib --crate-type cdylib`, pins the actual
Cargo-reported `.so` under the lease, and retains Cargo logs, depfiles, selected
source copies and hashes. Before Cargo it preserves any previous unversioned
`.so` cache output byte-for-byte, then removes only those output names under the
lease. Cargo must regenerate the requested leaf; a Fresh leaf is refused.
Dependency rlibs, compiler flags and profiles are retained. This prevents a
different graph's same-name `.so` from being accepted through an old fingerprint.
Ordinary manifests and Android/JNI builders are unchanged.
The generated cdylib supplies no new C ABI or stable Rust dynamic ABI contract.
Windows/Android/macOS artifacts are not qualified by this Linux entry.
This entry uses the root workspace's selected feature graph; it does not select
arbitrary standalone dependency features with `-p`.

The relay CLI already existed in the pinned original and current fork; this
ticket does not describe it as restored. Its root-workspace build entry remains:

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/iroh-relay-cli -- \
  build --locked --offline -p iroh-relay --bin iroh-relay --jobs 2
```

The root already selects relay's `server` feature. Cargo refuses `--features`
for this dependency outside the workspace; the validated command uses that
existing root graph without attempting a feature override.

The current source inventory and detailed qualified checks are retained in
[distribution evidence](../evidence/vendor-distribution-2026-10-19/README.md).
The [October 12 audit](../evidence/fork-change-audit-2026-10-12/README.md) remains
an unchanged historical snapshot; its deleted-target counts and pre-compatibility
API/default deltas are not current conclusions. The new audit compares all nine
forks to the same fixed original artifacts and records source hashes and complete
diffs. Source statistics do not prove full platform or runtime compatibility.

## Qualified checks

| Entry | Actual result and limit |
| --- | --- |
| Restored Noq examples | All five compile; three unchanged main programs establish five connections and exit, then fixed UDP ports rebind. Client/server help entries pass; no file-serving runtime is inferred. |
| Extracted source-package consumer | Co-distributed Noq and Noq-proto compile unchanged `connection` and `single_socket` examples with the frozen local-record lock; four connections and port rebinding pass. |
| Noq integration target | The original ignored 50-connection, 1 MiB data/CRC case is explicitly invoked and passes. Its teardown is the published test's runtime/process teardown, not a new SDK actor-join contract. |
| Declared benchmarks | Noq benchmark and Noq-proto `send_buffer` compile and pass Criterion smoke mode. No throughput or latency result is claimed. |
| Linux cdylibs | Actual iroh and relay `.so` files are generated, pinned and loaded. A deliberately invalid same-name cache output is preserved and replaced; the regenerated library equals the earlier good pin, with one leaf rebuilt and dependencies Fresh. |
| Precondition failures | Missing declared target content is refused before Cargo. Required root manifest/lock/configuration and declared library/build-script inputs must exist before `prepare` creates a snapshot; ten isolated missing-input cases pass under normal Python and `python -O`. Source, lock and protected-artifact mismatches after preparation also stop before Cargo in both modes. |
| Existing relay CLI | The root's already-present server feature builds the real `iroh-relay` binary; its help entry exits normally in an isolated container. This is not a new relay-service runtime qualification. |
| Current Native/Browser SDK | Fresh owned loopback TLS/WSS and relay run: restricted request, one approval, both materials durably saved, both identities reopened, member PONG and normal cleanup pass. The prior raw SDK/WASM pins are reused after checking every selected source/artifact hash; no SDK rebuild is claimed. |

All owned runtime containers are gone; generated TLS keys and private NSS state
are removed after qualified actor shutdown. SDK private receipts and actor/store
logs are excluded from public evidence. The optional PQ runtime, other providers,
Windows/Android/macOS artifacts, public networking and physical-device matrices
remain outside this ticket's qualified set. The missing offline dependency was
fetched at its published lock version/checksum, without re-resolving the graph.
