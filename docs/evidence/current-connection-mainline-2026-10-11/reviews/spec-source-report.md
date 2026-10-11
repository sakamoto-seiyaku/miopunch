# Spec review

Reviewed `29b5a2ceec62b37d6870add3744045a15cd858bb...78da27b2` against the actual M84 Plane snapshot `/tmp/miopunch-ask-matt-next-20261011/M84.json`, ADR 0009, current design notes, and retained receipts. Read-only review; no compilation or public tests repeated.

**Source verdict: zero required source fixes and zero scope-creep findings.** The four changed files correct probe budget attestation and permit explicitly selected 12s/60s callers. Both Browser roles receive the selected value once; Native ready/reopen/member reports must preserve their saved policy. The SDK and product Quality policy are unchanged. This fits “通过实际 SDK/公开操作入口运行” and ADR 0009's explicit shorter budgets with one absolute deadline.

**Whole-M84 acceptance gaps, separate from the completed finite selection:**

1. **Current directional and physical qualification remains partial.** Spec: “Native→Native、Browser→Native、Native→Browser、Browser→Browser及其反向均实际记录…” and “输出公开设施、完整受限引导、业务路径和无 VPN NAT 四层资格”. Current Host/Pixel Browser bidirectionality and Native/Browser relay business pass, but every current run retains `physicalWanQualified=false`; controlled Host RTC and loopback Native fallback cannot certify physical WAN. Historical OP13 and eight-case records belong to earlier artifact epochs and do not fill the current full matrix.
2. **Current nonempty Replica evidence is missing.** Spec: “选择已验证方向完成首次准入/重开/非空 replica”. Current relay and qualified IRC stories demonstrate actual approval, material saving, reopen and member PING/PONG. They do not execute nonempty Replica verification/save/apply/persistence. Earlier Replica results remain historical.
3. **Complete terminal cleanup remains unproven.** Spec: “清理本票拥有的连接、PC、pump、队列、listener、临时进程及转发；记录即时与晚期 socket 回收”. Public relay's late sample retains two CLOSING sockets, four SDK callbacks and `terminal-event-pending`; full SDK terminal-task retirement is false. Carrier repair and RTC no-path each retain one pending MQTT socket at the late sample. Natural Native exit and owned browser/tab retirement do not establish full pre-tab SDK terminal-task completion.

The requested finite integration/review work can finish with these disclosed gaps; M84/M69 cannot be marked Done. Qualified IRC uses explicit 60s caller/8s assembly/30s queue and does not certify the full default profile. No TURN, UI or new product behavior was introduced.
