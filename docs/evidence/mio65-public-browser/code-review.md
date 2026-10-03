# MIOPUNCH-65 independent code review

Fixed point: `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`; selected owned WIP before commit. Exact five-path scope and diff: `/tmp/mio65-review-scope.json` and `/tmp/mio65-review.patch`. Concurrent MIOPUNCH-64 and protected user WIP are excluded. No source changed after these reviews.

Both independent reviewers used GPT-6.1 Sol HIGH. Root confirmed actual `serviceTier: priority` through read-only runtime readback, not a supported-tier listing. Canonical paths: `/root/implement_65_browser_sol61_high/review65_standards_sol61_high` and `/root/implement_65_browser_sol61_high/review65_spec_sol61_high`. Receipts: `/tmp/miopunch-priority-review65_standards_sol61_high.json` and `/tmp/miopunch-priority-review65_spec_sol61_high.json`.

## Standards

# MIOPUNCH-65 Standards review

Scope: `/tmp/mio65-review.patch`, selected WIP against `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`; no ticket commits yet. Reviewed the five paths in `/tmp/mio65-review-scope.json`. Concurrent MIOPUNCH-64 and protected user WIP are excluded.

**Documented-standard findings: 0.**

- `src/bin/browser_network_native.rs`: added path observations and bounded fixture startup preserve the existing native public seams and membership gates. This is consistent with `CONTEXT.md` (“A discovery or relay facility does not grant either by itself”) and ADR 0004's independent admission-secret/member-authorization boundary.
- `browser-probe/harness/run-public-relay.mjs` and `scripts/check_browser_public_relay.py`: the selected relay remains explicit; persisted disable rejects startup without alternate access. This follows the product-direction draft §3–4 requirement that disabled official resources must not be silently re-enabled. The static server retains its documented delivery-only role.
- The qualification note and evidence README distinguish validation from product decisions, document foreground/browser/service limits, and retain Plane links without creating a work ledger. This follows `AGENTS.md` (“Do not turn research suggestions into product decisions”), `docs/agents/domain.md`, and `docs/agents/issue-tracker.md` (“Repository docs may retain evidence, acceptance criteria, and links”). Existing JSON corroborates the 334 ms disable, zero external attempts, same identity/member recovery and exit-0 cleanup.

**Optional heuristic: 1, parked and nonblocking.** Possible Duplicated Code: the official-region list is repeated in `run-public-relay.mjs:16` (`['aps1-1', 'use1-1', 'usw1-1', 'euc1-1']`) and `check_browser_public_relay.py:14` (`OFFICIAL_RELAYS = tuple(...)`). A future service-table change needs both guards updated. This is a judgement call, not a documented violation; independent entry-point validation explains the duplication. No refactor is requested under the user's completion priority.

All twelve baseline smells were considered; no other supported finding. Tool-enforced formatting/type rules were excluded. Read-only review used existing evidence; no tests, repository changes, staging, commits or tracker writes.

## Spec

# MIOPUNCH-65 Spec review

Result: **0 findings** across missing/partial requirements, unasked scope, and incorrect implementation.

Reviewed the supplied live-ticket HTML and the five-path WIP diff against `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`; no ticket commits exist yet. Shared MIOPUNCH-64 and unrelated WIP are excluded from this review.

“实际浏览器运行当前 BrowserNetwork，使用独立持久身份和 IndexedDB” is satisfied by the existing BrowserNetwork and a separate persistent Chromium profile. The static origin only distributes assets. The runner passes no custom CA, sets `ignoreHTTPSErrors: false`, and supplies no certificate-ignore argument. The selected hostname remains listed in [official source](https://github.com/n0-computer/iroh/blob/main/iroh/src/defaults.rs); [official documentation](https://docs.iroh.computer/add-a-relay) identifies these n0 public relays as development/testing facilities.

“端口连通或 WebSocket 打开不能单独作为完整成员通信成功” is satisfied by exact authorized echo responses, native selected-relay path observations with bidirectional bytes, and real native-source LogSync. The runner verifies the original nonempty signed-record body, both saved admission materials, and the same identity/record after refresh. Retained current-entry evidence records Chromium 153.0.8010.12, origin, timestamps, HTTPS results, and three actual WSS 101 sessions.

“仅阻断或禁用选定官方 relay，且不给未批准的替代设施” is exercised after the first complete check: explicit and persisted disable fail clearly, with disabled business controls, zero WSS/external attempts, and a measured 334 ms first failure. Explicit restore and reopen retain identity and the exact record and perform another real exchange. Unchanged member/request/record counts and one approval demonstrate no repeated Add, approval, or redemption.

No product scope expansion was found. Native changes add bounded fixture startup and transport observations. Original RED, public GREEN runs, affected checks, cleanup, and the separate self-hosted regression are retained. Tests were not rerun during this read-only review. The concurrent shared-core build context and unchanged slow NAT exclusion are explicitly disclosed.

Review aggregation, ticket commit, and final OS-temp handoff remain normal post-review completion steps, not behavioral defects.

Reviewer runtime readback: GPT-6.1 Sol, HIGH, actual `priority` service tier.

Standards: 0 documented violations, 1 nonblocking optional duplication heuristic (parked); Spec: 0 findings. No required fixes remain.
