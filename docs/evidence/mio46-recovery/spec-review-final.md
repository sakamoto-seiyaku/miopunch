# MIOPUNCH-46 supplemental independent Spec review

Reviewed only `95b536d0..3eb04819` in `src/mqtt_discovery.rs`, `src/network_recovery.rs`, `src/offline_replica.rs`, `scripts/check_network_recovery.py` and the recovery note, against the same complete ticket/parent specifications and accepted decisions as [the original review](spec-review.md).

**Remaining Spec findings: 0.** No missing/partial requirement, scope expansion or incorrect required behavior introduced by this maintenance.

The requirement “只把候选当线索；保留资源资格、阶段准入和真实身份核对” remains satisfied: the provider helper preserves the same predicate at both original authorization checkpoints. Recovery still requires Bootstrap admission, an IP data path and current membership before providing history; neither checkpoint was removed or weakened.

“将已验证的候选发现 profile 接入同一恢复流程与地址簿” remains unchanged. Legacy routes retain `MDR1` with sixteen bytes; configured origin-qualified routes retain `MDR2` with thirty-two bytes. `TaggedCandidate` names the existing type. The explicit private profile centralizes the format selection and rejects unsupported responder widths before broker access. Private discovery still decodes unsupported widths as invalid; its production callers use only the two supported widths. The note's broad early-rejection wording should be read as responder validation, not a new discovery capability.

The additional runner option only selects existing acceptance, legacy-profile, typecheck and formatting checks. It introduces no runtime behavior or product scope.

Reviewed `review-maintenance/results.json`: all twelve real acceptance checks pass, alongside both legacy bootstrap tests, all-targets/current hooks and formatting. The startup log explicitly confirms durable authorization after reopen. No successful checks were repeated during this review. Native Linux, owned MQTT and IPv4 UDP remain the qualification boundary; broader NAT/IRC/platform behavior remains unclaimed.
