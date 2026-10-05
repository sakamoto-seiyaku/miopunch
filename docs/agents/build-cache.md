# Rust build cache

Use one cache at the repository's `target/`. Native, WASM and Android artifacts
already occupy their own Cargo target subdirectories. A new verification output
directory is for receipts and selected immutable artifacts, not another Cargo
cache. Do not clone `target`, its `deps`, or its incremental directories.

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
Temporary test-only selectors preserve the existing lockfile's dependency
versions, sources, checksums and dependency rows. Change only their local package
record when necessary. Validate the declared source and lock graph in the same
driver that starts Cargo, and abort before compilation if any precondition fails;
do not regenerate a selector lockfile or continue after a failed validation.

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
Preserve selected `.d` inputs, source hashes, command exits and SDK artifact
hashes; a mutable Cargo cache is not an immutable evidence archive.

Inspect capacity before choosing cleanup:

```sh
rtk proxy python3 scripts/cargo_cache.py inspect
rtk proxy python3 scripts/cargo_cache.py cleanup-incremental
```

Incremental cleanup removes only top-level compiler IR caches and refuses while
this repository has live Cargo/rustc processes. It retains executable artifacts,
libraries, dependency information, toolchains and proof files. Do not run blanket
`cargo clean`, delete all `deps`, strip accepted originals, or delete another
agent's active target. Obsolete test executables require a separate reviewed list
with current and published artifact references exempted; keep dependency
libraries so cleanup does not trigger a complete dependency rebuild.
