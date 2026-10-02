# MIOPUNCH-48 Spec review

Reviewed committed diff `866736ead65f67bf6c0850dfed746637cf364545...5d77a8a8`, against the complete live MIOPUNCH-48 and parent MIOPUNCH-14 bodies saved in `/tmp/mio48-plane/`, current context/ADRs, the recorded member-relay decisions, implementation, tests, and evidence receipts. Unrelated working-tree documentation is excluded.

**Confirmed findings: 0.** No missing/partial ticket requirement, unrequested product behavior, or demonstrably incorrect implementation found.

The implementation separates explicit B data permission from signaling, preserves real A-C iroh TLS and endpoint identity, binds route source labels to authenticated outer members, rejects unknown/retired routes and nested circuits, and defines bounded queues, registrations, datagrams, rate budgets, and route lifetime. Existing sender qualification retires unavailable routes per original Connection; the recorded failure tests preserve the same Connection IDs and stream on allowed UDP backup or report clear sole-path failure.

Evidence contains thirteen ordered original acceptance cases plus one actual signaling-isolation case, fourteen maintenance checks and 184 affected short checks. The original IRC harness failure remains recorded; the continued receipt supplies real owned usable/limited servers and passes the remaining checks. No duplicate tests or slow unchanged NAT matrices were run for this review. Validation is limited to the documented Linux IPv4 controlled-carrier environment; browser runtime, arbitrary NAT, and security-audit claims are excluded.

**Prior observation addressed; no finding:** The final-source follow-up supplies only the data provider writer with the route enable-watch receiver. After each dequeue, the writer rejects a disabled or changed permission epoch before initiating a new frame. Signaling and client-side writers retain their existing behavior. An immediate disable/re-enable still retires the old epoch; already begun physical outer writes remain in flight. This directly addresses the earlier ready-select observation without claiming it was a reproduced bug or expanding scope. It implements ticket line 23, “关闭 B 中继权限，停止新转发且不转移成隐式信令业务。”

Reviewed the narrow `69b5e511..5d77a8a8` follow-up. Its saved receipts confirm exactly one real priority, backup-disabled, and live-signaling test each, plus current all-target compilation and formatting. No new tests were needed for this review. Final assessment remains **0 confirmed Spec findings**.
