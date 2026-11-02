# Current fork source inventory

2026-10-12, repository HEAD `5beb2c79` (the full revision is recorded in
`inventory.json`). This is a read-only comparison, not a build, runtime test,
compatibility certification or implementation decision.

[inventory.json](inventory.json) records each dependency's original version,
archive SHA-256, VCS provenance when packaged, changed file hashes and line
counts. [summary.tsv](summary.tsv) is the compact source summary. The explanatory
report is [the fork change audit](../../notes/2026-10-12-iroh-p2panda-fork-change-audit.md).

The baseline is the original crates.io package for iroh, iroh-relay, noq,
noq-proto, netwatch, portmapper and p2panda-sync. For p2panda-auth and
p2panda-net it is the previously verified official p2panda archive at commit
`083b48215964e92a564f3c83a1c607b00e94aa64`; its SHA-256 was rechecked before
extraction. Original archives were extracted into a fresh temporary directory;
the cached registry source trees were not assumed immutable.

Counts come from `git diff --no-index --numstat` for each original/current
file. Unchanged upstream files are excluded from the change count. The
`rust_runtime_or_mixed` category means Rust files under `src/` outside known
standalone test modules, **including inline tests, comments and formatting**.
It is not a production-code line count. Standalone tests/examples, build files,
manifests, lockfiles, license additions, provenance and other packaging changes
are separate categories. A removed dependency-local Cargo.lock is not a removed
runtime implementation; the root Cargo.lock remains the application lockfile.

The audit does not compare against the newest upstream release, estimate
throughput from source size, or certify untested platform/direction combinations.
Exact source and behavior findings are in the report rather than inferred from
the number of changed lines.
