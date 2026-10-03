# MIOPUNCH-65 Standards review

Scope: `/tmp/mio65-review.patch`, selected WIP against `a6fa18371b5342609a0fe2c13fdc11a80c140a3b`; no ticket commits yet. Reviewed the five paths in `/tmp/mio65-review-scope.json`. Concurrent MIOPUNCH-64 and protected user WIP are excluded.

**Documented-standard findings: 0.**

- `src/bin/browser_network_native.rs`: added path observations and bounded fixture startup preserve the existing native public seams and membership gates. This is consistent with `CONTEXT.md` (“A discovery or relay facility does not grant either by itself”) and ADR 0004's independent admission-secret/member-authorization boundary.
- `browser-probe/harness/run-public-relay.mjs` and `scripts/check_browser_public_relay.py`: the selected relay remains explicit; persisted disable rejects startup without alternate access. This follows the product-direction draft §3–4 requirement that disabled official resources must not be silently re-enabled. The static server retains its documented delivery-only role.
- The qualification note and evidence README distinguish validation from product decisions, document foreground/browser/service limits, and retain Plane links without creating a work ledger. This follows `AGENTS.md` (“Do not turn research suggestions into product decisions”), `docs/agents/domain.md`, and `docs/agents/issue-tracker.md` (“Repository docs may retain evidence, acceptance criteria, and links”). Existing JSON corroborates the 334 ms disable, zero external attempts, same identity/member recovery and exit-0 cleanup.

**Optional heuristic: 1, parked and nonblocking.** Possible Duplicated Code: the official-region list is repeated in `run-public-relay.mjs:16` (`['aps1-1', 'use1-1', 'usw1-1', 'euc1-1']`) and `check_browser_public_relay.py:14` (`OFFICIAL_RELAYS = tuple(...)`). A future service-table change needs both guards updated. This is a judgement call, not a documented violation; independent entry-point validation explains the duplication. No refactor is requested under the user's completion priority.

All twelve baseline smells were considered; no other supported finding. Tool-enforced formatting/type rules were excluded. Read-only review used existing evidence; no tests, repository changes, staging, commits or tracker writes.
