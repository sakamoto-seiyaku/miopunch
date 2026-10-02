# Standards review — MIOPUNCH-43

Reviewed independently by GPT-6.1 Sol HIGH Fast against staged owned work from
b36ad3d98d7ac07458170ed89408f45190c91948. No tests rerun or files edited.

No documented-standard breaches found in the initial snapshot or targeted fix.
Bilateral observation, permission and validation results remain on the protected
stream; the protocol change and evidence boundaries are documented consistently
with ADR0005 and the requirement to record extensions.

One nonblocking judgment call: **possible Duplicated Code** in
`tests/tcp_punch.rs`. Branches repeat peer-target construction despite the
existing `peer_target()` helper, and repeat
`Command::new("ss").args(["-Htan", "state", "syn-sent", ...])` with command-success
checks. Reusing the target helper and extracting socket observation would reduce
maintenance repetition while preserving scenario-specific assertions.

The new observer-failure and candidate-denial fixtures each isolate their
intended condition. Documentation distinguishes compiler failure from behavioral
GREEN. Post-fix final suites were still running when the reviewer read them;
actual suite completion is established by their results.json, not this review.

Standards: **0 hard violations; 1 existing nonblocking smell finding.**
