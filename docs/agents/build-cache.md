# Rust build cache

Use one cache at the repository's `target/`. Native, WASM and Android artifacts
already occupy their own Cargo target subdirectories. A new verification output
directory is for receipts and selected immutable artifacts, not another Cargo
cache. Do not clone `target`, its `deps`, or its incremental directories.

These requirements also cover throwaway drivers in `/tmp`, temporary selector
manifests, worktrees and delegated work. The invocation's location does not
justify a new cache. Verification runners use `CheckBuilds` from
`scripts/check_build_cache.py`: compile the batch's selected targets under the
shared lease, pin only needed outputs, then run scenarios against those pins.
Reuse within a runner is in memory; Cargo owns invalidation between invocations.

The root package contains the SDK and its test CLI. The 17 auxiliary programs
are explicit targets of `miopunch-native-tools`, outside the default root test
selection. Build them with `-p miopunch-native-tools --bin <name>`; the shared
command helper also selects their package for existing build drivers. Workspace
checks include the tools and forward the existing hook features. Binary unit
tests remain enabled for the programs that contain them.

The standalone wasm-bindgen 0.2.129 tool lives at
`.artifacts/tools/wasm-bindgen/0.2.129/wasm-bindgen`, backed by the shared immutable
store. Browser builders and fixtures prefer this path; `receipt.json` beside it
retains the original release and binary identity. It is independent of Cargo's
cache and historical run directories.

CLI integration tests accept `MIOPUNCH_TEST_CLI` for a verified pin and otherwise
retain Cargo's normal `CARGO_BIN_EXE_*` behavior. Browser fixtures and kernel
labs receive the selected artifact paths from their runner. `--strip-debug`
transforms an owned deployment copy and removes it after its runtime owner ends.

Run builds from the repository root through the cache entry point:

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/my-build-receipt -- \
  build --locked --offline --features rtc-probe --lib --jobs 2

rtk proxy python3 scripts/cargo_cache.py run --output /tmp/my-test-build-receipt -- \
  test --locked --offline --features rtc-probe --test my_test --no-run --jobs 2
```

The entry point serializes compilation, checks free space and cache size, and
records Cargo's actual `fresh`/rebuilt artifact counts and raw logs. Its defaults
require 10 GiB free and bound the shared cache to 96 GiB before starting another
build. It refuses a per-case target unless `--isolation-reason` explicitly names
the different source or toolchain requirement. Isolation does not permit copying
the existing target tree. Preserve the reason and remove the regenerable isolated
cache when its required artifact has been pinned and its owner has no live jobs.

Keep `.cargo/config.toml` dev/test profiles unchanged between routine commands.
They use line tables and disable Rust incremental IR, while retaining Cargo's
compiled dependency cache. Unchanged dependencies remain reusable. Do not vary
`CARGO_PROFILE_*`, `CARGO_INCREMENTAL`, or Rust flags to speed up one isolated
check. Distinct platforms, intentional features and source changes can require
compilation; repeated identical inputs should report zero rebuilt artifacts.
The entry point rejects environment Rust flags and compiler wrappers as well as
dev/test profile overrides. A required alternative compiler setup needs its own
documented workflow, rather than changing routine cache inputs.
Prefer declared Cargo package/target selections over temporary manifests.
Specialized frozen-SDK consumers retain their verified dependency and lock
records. Validate source, lock, tool and artifact preconditions before Cargo;
abort on failure rather than resolving another graph or continuing.

`build_android_pair.py` defaults to the same cache and holds its lease through
compilation and artifact receipts. Its Windows-NDK fallback linker lives at a
stable, toolchain-identified path under `target/.miopunch-toolchains/android`.
New receipt directories do not change `CC`, linker or `-B` paths. Actual tool or
NDK revision changes get another toolchain identity.

The native library emits only `rlib` in ordinary builds. Cargo then stores each
dependency graph in a distinct hashed library file. Do not restore the mixed
`rlib`/`cdylib` default: it emitted an unversioned dependency library that one
feature/test graph could overwrite while another graph remained marked Fresh.
The Android builders generate JNI separately with `cargo rustc --lib
--crate-type cdylib`, using the same feature set, target, toolchain and cache
lease as the CLI build. The receipt records that command and its separate exit.
The older `build_android_node.py` delegates to the pair builder while preserving
its original non-RTC CLI feature set. JNI `.so` remains a mutable build output;
pin or deploy it under the lease before another JNI graph replaces it.

```sh
rtk proxy python3 scripts/build_android_pair.py \
  --ndk /path/to/ndk --output /tmp/android-build-receipt --bin browser_wss_member_native
```

For a batch of runtime cases, build each required SDK graph once and reuse its
verified, pinned artifacts. Rebuild only for a selected source, toolchain,
feature, profile or platform change. Changing a JavaScript-only capability
snippet requires its WASM/bindgen refresh, not an unrelated Native rebuild.
Check reusable artifacts and the batch's storage budget before starting; a brief
declaration is sufficient, not another planning or receipt framework. A new issue
number or output directory does not change a graph. The root package's CLI
supports its integration tests; auxiliary binaries belong to the tools package.
Select required targets and account for dev-dependency features. Once a test
executable is pinned, runtime retries verify and reuse that pin.
Investigate unexpected dependency rebuilds before starting another case. Do not
prune a graph's executable after each case and then rebuild it for the next case;
pin once and use the verified pin. Compilation failures with zero artifact
messages are not cache-hit evidence.
Preserve selected `.d` inputs, source hashes, command exits and SDK artifact
hashes; a mutable Cargo cache is not an immutable evidence archive.

## Native toolchain ABI boundary

The Tauri Linux builder uses a pinned Ubuntu 24.04/WebKitGTK image whose host
build scripts and proc macros can require GLIBC 2.39. The ordinary host is
Debian with GLIBC 2.36. An explicit Rust `--target` does **not** isolate Cargo's
host artifacts under `debug/`; sharing those host outputs across these systems
can make a later Windows/WASM/native build execute an incompatible cached ELF.

`build_desktop_host.py` therefore uses one stable image-identified cache leaf at
`target/.miopunch-toolchains/tauri-linux/<image>/cargo-target`, under the same
outer repository cache lease and total disk budget. This is the documented
different-toolchain isolation exception, never a per-run cache. Do not copy a
target tree into it. Reuse qualified Linux pins until a real graph input changes.
The normal Debian/native, Windows cross, WASM and Android graphs retain their
existing shared locations and profiles.

The first Ubuntu graph exposed this incompatibility during Windows integration.
Its accepted executable is retained. Repair of the existing host cache is limited
to an inventoried set of incompatible host fingerprint metadata, archived before
invalidation and checked against host-visible processes. Dependency libraries,
executables and immutable pins are preserved; Cargo regenerates the invalid
entries with the baseline toolchain when needed. Never repair this by repeatedly
cleaning whichever package fails next, changing Rust flags, or deleting `deps/`.

## Immutable artifact storage

Keep one byte copy of an identical SDK output across receipt directories:

```sh
rtk proxy python3 scripts/pin_artifact.py target/debug/my-sdk /tmp/my-run/pinned/my-sdk
```

Build drivers import `pin_artifact` and call it while they hold the Cargo lease.
The helper hashes and copies the mutable source into `.artifacts/sha256/` once,
verifies it, makes it read-only, and hardlinks per-run paths from that independent
object. Subsequent identical outputs reuse that object. It never hardlinks to
the mutable Cargo inode, overwrites an existing pin, or silently falls back to a
full copy on another filesystem. It rejects a changed source, corrupt object,
insufficient free space (10 GiB reserve), or growth past the default 32 GiB store
budget. Review retention before changing that budget; do not create per-ticket
stores to bypass it. Include this helper in the build driver's frozen inputs.

Pins are immutable even if their original mode allowed writes. Never overwrite,
chmod or strip a pin or store object: every link shares its inode. Make a separate
owned deployment copy for transformations, and remove that copy when its runtime
owner has finished. Keep per-run source/lock hashes, selected inputs and logs,
and record the original receipt and hash when reusing a prior build. Do not pin
every Cargo `Fresh` executable when the case only needs one selected program.

The store is an evidence store, not compiler cache, and is excluded from Git.
Garbage collection requires a reference/retention review; `cargo clean` cannot
manage it. A store object whose receipt links remain needs no additional copy.

## Cleanup

Inspect capacity before choosing cleanup:

```sh
rtk proxy python3 scripts/cargo_cache.py inspect
rtk proxy python3 scripts/cargo_cache.py cleanup-incremental
```

Incremental cleanup removes only top-level compiler IR caches and refuses while
this repository has live Cargo/rustc processes. It retains executable artifacts,
libraries, dependency information, toolchains and proof files. Do not run blanket
`cargo clean`, delete all `deps`, strip accepted originals, or delete another
agent's active target. Remove idle executables only from an explicit reviewed list, retaining useful
dependency libraries so cleanup does not trigger a complete rebuild. Retain
programs, tools and pins for current consumers, actual publication or required
reproduction. A newest-per-family rule, a historical log/receipt mention or a
different fingerprint is not a retention reason. Keep original logs and provenance
without retaining every regenerable executable. Consolidate immutable copies
while preserving paths still required by consumers; never hardlink a pin to
mutable Cargo output. Check host-visible processes/open files first: sandbox
`/proc` can hide other agents. Record the exact deletion list, exclusions and
actual freed blocks; reuse valid checks.

The 2026-10-21 cleanup removed 220 reviewed obsolete shared-cache executables
(28.24 GiB) and consolidated 59 duplicate immutable SDK copies (3.60 GiB). It
retained dependency libraries, depfiles and all SDK paths. A repeated controlled
library build reported 527 Fresh artifacts and zero rebuilt artifacts. See the audit receipts under
`/tmp/wsl-disk-audit-20261021/` for the exact exclusions and hashes. The persistent
policy is to budget and reuse graphs/pins; temporary audit paths are not a new
build dependency.
