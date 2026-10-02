# MIOPUNCH-41 independent code review

The actual code-review skill used two independent GPT-6.1 Sol HIGH agents.
Fixed baseline `afd50dd830af0364b0472463eba6872de509be25`; only41 snapshot `33889f60`
in an owned temporary no-checkout worktree. Main branch/index stayed untouched
during review. Runtime source, tests and runners did not change after the reviewed
snapshot; only these reports and their index links follow. Optional fixture
refactoring suggestions are recorded as non-blocking maintenance judgments.

## Standards

Hard documented-standard violations: **0**. The diff respects the current baseline, records concrete integration choices and limitations, and documents both vendored deltas. No conflict found with AGENTS.md, docs/agents/domain.md, the current transport decisions or relevant ADRs.

Optional baseline smells:

- **Possible Duplicated Code — tests/tcp_portmap.rs:201, 303, 424, 471.** The client round trip repeats `send.write_all(b"authorized via actual TCP mapping")` followed by the same response assertion; the server exchange repeats separately. Small client/server exchange helpers would keep payload and authorization checks consistent while leaving scenario orchestration visible.
- **Possible Repeated Switches — tests/tcp_portmap.rs:398 and 482.** Both branches repeat `"expired" | "changed" | "restart" | "renewed" | "renewal-address-failure"` to decide whether to verify the retained session; scripts/tcp_portmap_lab.py:194 carries the same classification. A named Rust predicate would remove the direct duplication. Keep that classification aligned with the Python lab when adding scenarios.

These are maintainability judgments, not blockers or required design changes. No new tests were run.

Standards total: **0 hard violations, 2 optional smells**; the largest concern is duplicated fixture exchange logic.

## Spec

Spec review: **0 findings** against MIOPUNCH-41 and relevant MIOPUNCH-14 requirements.

- **Missing or partial requirements:** None found within the ticket’s declared scope. Independent TCP listener ownership and existing candidate/connection integration appear in `src/resources.rs` and `src/tcp_transport.rs`. Recorded positive evidence exercises real TCP forwarding, node authentication, manual admission, persisted materials and member data exchange.
- **Unasked behavior or scope creep:** None found. Binding/shutdown hooks and the NAT-PMP release correction support the requested mapping lifecycle. PCP/UPnP, other platforms, hardware routers and TCP punching remain explicitly unclaimed.
- **Apparently implemented but wrong behavior:** None found. Candidate updates follow the existing lease watcher; recorded failure, replacement and restart cases separately check established communication and fresh dial outcomes. Completed release retains the allocated protocol in `vendor/portmapper/src/nat_pmp.rs`; priority evidence preserves the independent same-port UDP mapping and foreign TCP/UDP rules. Advanced-builder cases verify saved mapping and global TCP disables.

Reviewed the pinned diff, relevant implementation/test/lab code and accepted evidence indexed in `docs/evidence/mio41-tcp-portmap/README.md`. This was read-only inspection; no tests were rerun. Evidence supports controlled Linux IPv4 NAT-PMP, with the documented broader capability limits.

Summary: Standards0 hard violations/2 optional smells (largest: duplicated fixture exchanges); Spec0 findings/no issue.
