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
