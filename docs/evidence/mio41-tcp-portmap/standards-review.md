# Standards review

Independent GPT-6.1 Sol HIGH; snapshot `33889f60`, baseline `afd50dd830af0364b0472463eba6872de509be25`.

Hard documented-standard violations: **0**. The diff respects the current baseline, records concrete integration choices and limitations, and documents both vendored deltas. No conflict found with AGENTS.md, docs/agents/domain.md, the current transport decisions or relevant ADRs.

Optional baseline smells:

- **Possible Duplicated Code — tests/tcp_portmap.rs:201, 303, 424, 471.** The client round trip repeats `send.write_all(b"authorized via actual TCP mapping")` followed by the same response assertion; the server exchange repeats separately. Small client/server exchange helpers would keep payload and authorization checks consistent while leaving scenario orchestration visible.
- **Possible Repeated Switches — tests/tcp_portmap.rs:398 and 482.** Both branches repeat `"expired" | "changed" | "restart" | "renewed" | "renewal-address-failure"` to decide whether to verify the retained session; scripts/tcp_portmap_lab.py:194 carries the same classification. A named Rust predicate would remove the direct duplication. Keep that classification aligned with the Python lab when adding scenarios.

These are maintainability judgments, not blockers or required design changes. No new tests were run.

Standards total: **0 hard violations, 2 optional smells**; the largest concern is duplicated fixture exchange logic.
