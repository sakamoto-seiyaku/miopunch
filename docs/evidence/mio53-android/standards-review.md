## Standards

Reviewed `git diff db528ca1e6d0f36941ddb00687d838e9b1b21504...HEAD`, with HEAD `19611e482019e4418526c9188b3d76ce81542391` (one commit: `MIOPUNCH-53 run Android management and recover native lifecycle`).

**Documented-standard violations: none found.** The change follows AGENTS.md's current-baseline boundary, docs/agents/domain.md's shared vocabulary and accepted-decision constraints, and docs/agents/issue-tracker.md's allowance for repository evidence rather than an open-work ledger. The lifecycle shell delegates to the existing maintained node; Android publication preserves no-replace semantics. The delivery note explicitly separates measured shell/LAN behavior from unqualified installed-app capabilities. Relevant ADRs 0001–0004, 0007 and 0008 are not contradicted.

**Optional heuristic — possible Data Clumps / Mysterious Name:** `scripts/check_android_management.py:107`, consumed at lines 112 and 136, returns `admin, ar, client, cr, owner, member, invite, requests`. The eight positional values repeatedly travel together; abbreviations `ar` and `cr` require reading setup to discover that they are ready responses. A small named fixture result with descriptive fields would reduce positional coupling and clarify which process, ready response and directory belong together. This is a maintainability suggestion, not a documented violation or demonstrated bug; defer it under the current completion priority.

Thin delegating platform methods are intentional, so the Middle Man heuristic is suppressed. Tool-enforced formatting and raw evidence whitespace are excluded. No source or Plane edits and no tests rerun.

Standards total: 0 hard violations, 1 optional heuristic finding; no blocking issue found.
