# MIOPUNCH-48 two-axis code review

Fixed point: `866736ead65f67bf6c0850dfed746637cf364545`.
Reviewed implementation commits: `69b5e511` and `5d77a8a8`.
Independent GPT-6.1 Sol HIGH priority/Fast agents used `fork_turns=none`.
Full diff: `rtk proxy git diff 866736ead65f67bf6c0850dfed746637cf364545...HEAD`.
Source and current live specification are recorded in the separate reports.

## Standards

Reviewed committed diff `866736ead65f67bf6c0850dfed746637cf364545...69b5e511`
(`MIOPUNCH-48: add explicitly enabled one-hop member data relaying`).
Unrelated dirty documentation and separately maintained Cargo configuration
were excluded. No implementation edits or tests were performed by this reviewer.

## Documented standards

No hard violations found. Checked `AGENTS.md`, `/home/js/.codex/RTK.md`,
`CONTEXT.md`, current design entry points, `docs/agents/{domain,issue-tracker,triage-labels}.md`,
and relevant ADRs 0001/0003/0004/0005. The relay note explicitly records current
choices and qualified evidence; archived workflows and research suggestions are
not promoted to implementation requirements. Signaling, data paths, membership,
and endpoint identity retain the documented distinctions. Existing tooling
receipts cover formatting and compilation; tooling-enforced issues were omitted.

## Smell baseline

One nonblocking judgement call, parked under the user's architecture-review
instruction:

- **Possible Duplicated Code**, `src/resources.rs:483` and `src/resources.rs:503`:
  both changed hunks repeat the recognized-carrier predicate,
  `matches!(addr.id(), crate::tcp_transport::DATA_TRANSPORT_ID | crate::member_transport::DATA_TRANSPORT_ID)`,
  followed by `self.config.allows_custom(...)` (the second uses `address`).
  This extends pre-existing TCP duplication; no incorrect behavior was found.
  A shared recognized-data-carrier predicate could prevent future advertisement
  and connection filtering from diverging. This is optional cleanup, not a new
  acceptance requirement or a reason to delay completion.

The other eleven baseline smells produced no actionable findings. Thin data
wrappers preserve deliberate public permission boundaries while reusing the
existing packet engine; they are not treated as an actionable Middle Man smell.

**Totals:** 0 documented-standard breaches; 1 parked heuristic; no blocking
Standards finding.

## Final-source follow-up

Reviewed `69b5e511..5d77a8a8` for the data-provider egress consent recheck and
its recorded limit. The optional watch receiver serves this concrete permission
boundary; signaling and client writers supply `None`. No additional standards
breach or actionable baseline smell was found. Separate ingress/egress checks
are appropriate at their respective boundaries. No tests rerun. Final totals
remain 0 hard violations and 1 parked heuristic.

## Spec

Reviewed committed diff `866736ead65f67bf6c0850dfed746637cf364545...5d77a8a8`, against the complete live MIOPUNCH-48 and parent MIOPUNCH-14 bodies saved in `/tmp/mio48-plane/`, current context/ADRs, the recorded member-relay decisions, implementation, tests, and evidence receipts. Unrelated working-tree documentation is excluded.

**Confirmed findings: 0.** No missing/partial ticket requirement, unrequested product behavior, or demonstrably incorrect implementation found.

The implementation separates explicit B data permission from signaling, preserves real A-C iroh TLS and endpoint identity, binds route source labels to authenticated outer members, rejects unknown/retired routes and nested circuits, and defines bounded queues, registrations, datagrams, rate budgets, and route lifetime. Existing sender qualification retires unavailable routes per original Connection; the recorded failure tests preserve the same Connection IDs and stream on allowed UDP backup or report clear sole-path failure.

Evidence contains thirteen ordered original acceptance cases plus one actual signaling-isolation case, fourteen maintenance checks and 184 affected short checks. The original IRC harness failure remains recorded; the continued receipt supplies real owned usable/limited servers and passes the remaining checks. No duplicate tests or slow unchanged NAT matrices were run for this review. Validation is limited to the documented Linux IPv4 controlled-carrier environment; browser runtime, arbitrary NAT, and security-audit claims are excluded.

**Prior observation addressed; no finding:** The final-source follow-up supplies only the data provider writer with the route enable-watch receiver. After each dequeue, the writer rejects a disabled or changed permission epoch before initiating a new frame. Signaling and client-side writers retain their existing behavior. An immediate disable/re-enable still retires the old epoch; already begun physical outer writes remain in flight. This directly addresses the earlier ready-select observation without claiming it was a reproduced bug or expanding scope. It implements ticket line 23, “关闭 B 中继权限，停止新转发且不转移成隐式信令业务。”

Reviewed the narrow `69b5e511..5d77a8a8` follow-up. Its saved receipts confirm exactly one real priority, backup-disabled, and live-signaling test each, plus current all-target compilation and formatting. No new tests were needed for this review. Final assessment remains **0 confirmed Spec findings**.

Standards: 0 hard violations, 1 parked nonblocking possible duplication; Spec: 0 confirmed findings, no remaining confirmed issue.
