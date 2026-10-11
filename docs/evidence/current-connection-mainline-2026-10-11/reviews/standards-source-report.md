## Standards

Reviewed `29b5a2ceec62b37d6870add3744045a15cd858bb...78da27b2` (`test(MIOPUNCH-84): attest actual caller budgets in SDK probes`), comprising four source/test files. No documented-standard violations or actionable baseline smells found.

- `browser-probe/harness/mio75-browser-pair-page.js:224–245`: `callerBudgetMs: entry.callerMs` records the same explicit budget passed to the actual connect/accept attempt. The supported 12,000/60,000 ms choices remain bounded; each attempt owns one SDK handle and retains cancellation/late-return retirement.
- `scripts/check_unified_browser_browser.py:445–475,765–769`: the existing CLI default remains 12,000 ms. Explicit 60,000 ms cases pass that value to both SDK roles and compare returned budget/elapsed evidence against it. The longer controller observation window does not recreate an SDK deadline. Pre/post selected-input, artifact and controller guards remain intact.
- `scripts/check_current_browser_relay.py:123–133,376–381,447–453,598–602`: Browser maintenance is explicitly configured to the reported 60,000 ms. Native evidence separately checks positive integer policy equality across initial readiness, reopen and acceptance, preserving shorter saved policies. `scripts/test_current_browser_relay_budget.py` includes rejection cases for clock changes, unknown values and booleans.

These changes comply with `docs/agents/build-cache.md` (pinned selected inputs/artifacts, precondition failure before execution), `CONTEXT.md` and ADR 0009 (one original caller deadline, saved shorter policies), and `docs/agents/domain.md` (current baseline; no historical workflow imposed). No Rust/build/profile/lockfile edits occur in this diff.

Read-only receipt inspection confirms current MQTT and public-relay success uses actual 60,000 ms budgets and retains `physicalWanQualified=false`. Relay evidence explicitly retains pending terminal-close callbacks and `allSdkTerminalTasksRetiredProven=false`; scoped cleanup success cannot establish complete graceful retirement. All 234 protected WIP hashes match the prior manifest. No public/build tests were rerun.

Findings: 0 documented-standard violations; 0 actionable smell judgments. Published dossier audit remains separate until its final candidate is available.
