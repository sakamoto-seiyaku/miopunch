# Disk and repeated compilation correction

The filesystem ran out of space before the new replica test could compile.
The failed cache copy was an environment failure, not an SDK protocol failure.
A read-only audit identified two avoidable sources of repeated compilation:
copying a new Cargo target per verification case, and creating Android linker
wrappers under each receipt directory. The latter changed Cargo's toolchain
inputs even when the actual NDK and sources stayed the same.

The repository now uses one `target/` cache, stable dev/test profiles with
incremental IR disabled, a serialized build entry point and stable Android
toolchain paths. A different target requires an explicit isolation reason.
The build entry point records actual Cargo JSON cache-hit counts. It requires
10 GiB free and checks the 96 GiB cache ceiling before starting compilation;
these are pre-start checks, not a hard quota during a build.
See [the build instructions](../../agents/build-cache.md).

Only regenerable incremental IR was removed from the main cache and two closed
worktree caches. The main cache's recorded removal was 78,746,034,176 bytes.
It measured 153,829,052,416 bytes before cleanup and 77,664,899,072 bytes after
the validation builds. The final read-only proof observed about 96.9 GiB free.
Cargo recreated four empty incremental directories; their contents were checked.
The proof verified 17 pinned SDK artifacts against their captured receipts.
Executable artifacts, dependency libraries, fingerprints and original evidence
were retained. Removal counts came from actual tool completion and were retained
in the root checkpoint; no machine-readable receipt existed at deletion time.

Each pair below used the same command and shared cache. All eight compile
commands exited zero. Platform, intentional feature and profile changes explain
first-run work; the second run measures reuse of identical inputs.

| Build | First rebuilt artifacts | Second fresh / rebuilt | Second seconds |
| --- | ---: | ---: | ---: |
| Native library | 11 | 527 / 0 | 0.470 |
| Android library | 394 | 598 / 0 | 0.487 |
| WASM | 9 | 296 / 0 | 0.378 |
| Selected replica test, compile only | 17 | 529 / 0 | 0.458 |

The installed Android NDK comparison used two new receipt directories and
confirmed the same linker path, contents and modification time. None of these
checks ran an SDK story, a phone test or a public network probe. The earlier
replica test ELF remains pinned and was not replayed.

The combined review covered this cache fix and the separate `createAnswer`
capability correction. Standards found a symlinked profile-ancestor cleanup
gap and a missing Android isolation-reason receipt field. Both were corrected.
The cleanup regression failed for host and target profile ancestors before the
fix and passed afterward; six cache tests and eight Android tooling tests pass.
The stable-linker regression passed in the original batch. Spec reported no
findings. Only the affected tooling was checked after the review corrections;
the four build pairs were not repeated.

Raw Cargo JSON, stderr, original audit measurements, cleanup proof and affected
tooling logs are retained as compressed originals under `raw/`. The package
contains no target clone, executable, WASM binary or source-tree copy. The
manifest records the hashes of the uncompressed originals and their archives.

The first SDK fixture compile after those cache-hit checks exposed another
boundary: the mixed `rlib`/`cdylib` default emitted an unversioned root dependency
library. The external test graph overwrote it, while the ordinary graph remained
marked Fresh. This produced real type-identity errors even though its dependency
libraries were still present. The first compile failure is retained separately
under `emitter-collision/`; the four earlier build pairs remain compile-only
cache-hit observations.

The default now emits only `rlib`, giving each graph a distinct hashed dependency
file. Android JNI explicitly emits `cdylib` after the CLI build. Both Android
entry points use the same builder and cache lease, and JNI failure retains its
separate exit without proceeding to Java packaging. Ordinary SDK source, roles,
wire and gates were unchanged by this repair.

The corrected Native fixture compiled with 526 Fresh artifacts and only the
root library and fixture rebuilt. The existing external test graph then compiled
with 527 Fresh and two rebuilt artifacts. Returning to the exact Native command
reported 528 Fresh / zero rebuilt in 0.454 seconds. The two hashed root libraries
coexist, the original library bytes remain unchanged, and forced metadata-only
type checks pass against both libraries and their respective direct iroh graph.
No SDK story or prior test case was replayed for this cache regression.

Actual Android JNI compilation reused 597 artifacts and rebuilt only the root
library. Its AArch64 binary exports the source-defined
`Java_MioPublicShell_session` entry. The initial export checker expected the old
`run` name and failed after successful compilation; the corrected assessment
reads the same outputs, without another build. A tiny independent emitter
regression also switched feature graph A → B → A and loaded the expected values
1 → 2 → 1. Explicit `cargo rustc` regenerates the cdylib primary target; JNI `.so`
must still be pinned or deployed while the build lease is held. This proves the
emitter behavior and exported ABI, not phone or JNI runtime operation.
