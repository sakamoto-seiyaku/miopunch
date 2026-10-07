# Standards review — final M77/M78 captured changes

Reviewed the 46-file captured owned diff against base `9c9f84e6653f6503f09cc6bcbd00013ceba6f00a` (no intervening commits). Scope, patch and all captured/current source hashes remained unchanged.

## Documented violations

**P2 — Two new test drivers bypass the mandatory shared Cargo cache entry point.**

[`scripts/check_contact_listener.py:66`](/home/js/Git/miopunch/scripts/check_contact_listener.py:66) executes `run('build', ['cargo', 'test', ...])`. [`scripts/check_member_contact_priority.py:87`](/home/js/Git/miopunch/scripts/check_member_contact_priority.py:87), line 89 and line 112 likewise start raw Cargo check/test commands through their generic subprocess runner. Neither driver acquires the cache lease or uses its command/environment helpers. `--locked --offline` and `-j 2` do not serialize these invocations with other repository builds or provide the capacity checks and actual Fresh/rebuilt receipts.

This conflicts with [AGENTS.md](/home/js/Git/miopunch/AGENTS.md)'s “Use the shared repository target/ through scripts/cargo_cache.py” and [docs/agents/build-cache.md:8](/home/js/Git/miopunch/docs/agents/build-cache.md:8), “Run builds from the repository root through the cache entry point”; lines 18–24 explain its serialization and disk guards. Route all compilation branches through that entry point, or its existing lease/command/environment helpers, and retain the raw compiler JSON needed to pin artifacts while still holding the lease.

This finding concerns the newly supplied drivers. The final Native/WASM SDK receipts already show successful guarded builds; it does not invalidate those immutable results or require another Rust build merely to repair Python driver dispatch.

## Optional judgement calls

No new optional refactor demand. Fowler smells were considered as heuristics, with the documented shared-coordinator/platform-adapter division taking precedence. Earlier parked P3 suggestions remain parked.

No other documented standards violation found. Formatting/compiler-enforced matters were excluded. Runtime version distinctions, local RTC versus public facility scope, UNOBSERVED counters and pending Browser socket tails are disclosed rather than promoted to stronger claims.

**Counts: 1 documented finding affecting 2 drivers; 0 new optional findings.**
