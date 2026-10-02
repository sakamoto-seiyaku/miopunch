# Standards review — MIOPUNCH-44

Reviewed committed diff `108f74c5bb9eaffa4d2eb6bce7ab77e6f06eb8d7...b0629e98ab167f42d9e34d2f901e3845ae212da9` using the code-review skill's Standards axis. Uncommitted user work is excluded.

**Documented-standard violations: none found.** Rules consulted: `AGENTS.md` (current baseline, explicitly recorded decisions, no legacy workflow), `docs/agents/domain.md` (CONTEXT vocabulary and accepted ADRs), and ADR 0005 (restricted side-channel business eligibility and independent membership checks). The implementation records its envelope and lifecycle decisions, reuses endpoint security, checks live Member scope, and keeps the new transport outside the business-carrier allowlist. Formatting/compiler-enforced concerns are excluded.

**Judgment calls — non-blocking smell findings:**

- **Possible Mysterious Name**, `src/native_network.rs:521`: the changed hunk reads `let builder = if automatic_mqtt {` followed by `self.gate.with_message_bootstrap().configure(builder).hooks(...)`. This flag now also enables member signaling and the shared path coordinator when MQTT resources are empty. Its MQTT-specific name obscures that broader responsibility. Rename it to reflect formal configured assembly or automatic signaling setup, including the existing call sites.
- **Possible Duplicated Code**, `src/member_transport.rs:270` and `:403`: both new transmit loops repeat `while let Some(packet) = ...recv().await { check_member(&gate, &connection)?; write_packet(&mut send, packet).await?; }`. Extract a small shared transmit helper so membership revalidation and write/error behavior cannot drift between forwarding and endpoint attachment. Keep their distinct receive logic and shutdown selection local.

No other baseline smells warrant a finding in the remaining changed hunks. The two recorded final receipts both report `passed: true`, with no failing check entries; unchanged slow NAT matrices were not rerun. This is a Standards review, not a new runtime/security audit or a Spec verdict.

Standards total: 0 hard violations, 2 non-blocking judgment calls; greatest concern is the duplicated membership-checked transmit loop.
