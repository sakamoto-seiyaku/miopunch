# MIOPUNCH-58 code review

Fixed baseline: `215e8d80fd115aba7f145f0cbd5efee41baa08d5` (MIOPUNCH-57).
Reviewed the ticket-owned working-tree diff and new tools, documentation and
saved evidence. Unrelated user-owned design drafts were excluded. Independent
Standards and Spec agents used GPT-6.1 Sol with high reasoning, as requested.

## Standards

No unresolved material Standards findings.

Read-only verification confirmed that matrix and capture checkers reproduce the
saved reports: 128 trials, baseline 0/64 and automatic 64/64. Capture-derived
packet and payload-byte medians match `measurements.json`. Costs distinguish
application primes, subsequent peer UDP traffic, and QAD traffic.

All 16 failure captures match their recorded budgets and show no business
delivery. Six boundary results and the recorded 189 passed / 3 ignored / 0 failed
test totals support the report. Documentation limits conclusions to controlled
APDM+APDF gateways and explains the 1+1 budget choice without claiming it was
faster.

Initial findings were repaired and re-reviewed: temporal packet deduplication
could merge real sends; missing end-of-window summaries could falsely mean zero
probes; default producer/verifier repeat counts differed; control-cost reporting
was duplicated. The revised capture check preserves two actual ingress primes
0.5 ms apart while excluding their outgoing capture copies, and retains the UDP
ports. Failure checks count actual captured sends, including partial windows.

## Spec

No remaining Spec findings.

Read-only checks confirm all 128 trials: baseline 0/64, automatic 64/64, all 16
ordered step combinations, both dial roles and two interleaved rounds. Packet
verification passed all 64 successful captures. Independent failure counts match
11 zero-prime cases and 5 bounded path misses. Six branch-boundary checks are
recorded.

Documentation limits the claim to controlled APDM+APDF, distinguishes application
primes from total UDP costs, and explicitly leaves unified dispatch and same
connection TCP fallback integration to later work.

The initial control-cost finding was repaired: endpoint event logs count five
actual sent/received coordination frames per successful side, including
promotion/commit, rather than four planned pre-path exchanges. Failed-path
completion has six actual sent frames per side. The final matrix and verifier
use the event counts.

Final findings: Standards 0; Spec 0. Neither axis has an unresolved material issue.
