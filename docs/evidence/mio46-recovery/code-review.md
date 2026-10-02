# MIOPUNCH-46 two-axis code review

Fixed point: `c6134e27098981fa9754b8265d47c91d083b729e`.
Initial implementation: `95b536d0f5284a86fb145a66d8333800dc7ebf17`.
Accepted maintenance: `3eb04819` (final runtime/test source).
Independent reviewers used GPT-6.1 Sol HIGH Fast/priority with isolated context.
Original uncommitted user documents and `.cargo/config.toml` are excluded.

## Standards

Reviewed `git diff c6134e27098981fa9754b8265d47c91d083b729e...HEAD` at
`95b536d0f5284a86fb145a66d8333800dc7ebf17` (one implementation commit).
Uncommitted user documentation and `.cargo/config.toml` are excluded.

Standards consulted: `AGENTS.md`, `/home/js/.codex/RTK.md`,
`docs/agents/domain.md`, `docs/agents/issue-tracker.md`, `CONTEXT.md`, current
design entry points, and ADRs 0004, 0005, 0007 and 0008. The recorded recovery
choices and evidence explain the supported implementation profile. Archived
Go/OpenSpec rules and tooling-enforced checks are excluded.

**Hard documented-standard violations: none found.** The change uses existing
identity, admission and durable-replica boundaries; explicitly records its
versioned discovery and recovery choices; and distinguishes source completion
from global freshness. Evidence documents failed intermediate checks and the
unchanged NAT-matrix exclusion without presenting them as successful coverage.

**Possible baseline smells — judgement calls, not hard violations:**

- **Primitive Obsession**, `src/mqtt_discovery.rs:65`, `:74`, `:84`:
  `Candidate<const N: usize = 16>` and
  `if N == 32 { TAGGED_RESPONSE } else { RESPONSE }` encode a discovery wire
  version indirectly through an arbitrary route length. Public `respond` accepts
  other lengths even though decoding supports only 16 and 32. Named legacy/tagged
  route types, or a constrained wire-profile type, would make the two supported
  representations explicit and prevent unsupported encodings. This is a small
  API-maintainability concern; current 16/32 call sites are supported.

- **Duplicated Code**, `src/offline_replica.rs:296` and `:312`:
  `!self.replica_allowed(connection) || (connection.alpn() ==
  crate::network_recovery::RECOVERY_REPLICA_ALPN &&
  !self.gate.allows_member_path_upgrade(connection))` repeats the provider's
  eligibility predicate. A named provider-eligibility helper would keep future
  policy edits consistent. Preserve both evaluations: the second correctly
  rechecks authorization after awaiting the request.

Review did not rerun tests. Receipts retain the ordered run's later resource
regression (`passed: false`), followed by successful continued-short and
reopened-startup runs. This report makes no additional runtime coverage claim.

**Total: 0 hard violations; 2 optional maintainability findings.**

### Independent final supplement

Independently reviewed `git diff 95b536d0..3eb04819` for the accepted maintenance
changes in `src/mqtt_discovery.rs`, `src/network_recovery.rs`,
`src/offline_replica.rs`, `scripts/check_network_recovery.py` and the recovery
note. The original review's current repository standards and exclusions apply.

**Both optional findings are closed.**

- **Primitive Obsession:** `DiscoveryProfile::{Legacy, OriginQualified}` now
  names the supported representations and centrally maps their route widths and
  response tags. `respond_live` rejects unsupported widths with `InvalidInput`
  before creating an MQTT client or subscribing; public `respond` delegates
  through this check. `TaggedCandidate` names the configured origin-qualified
  route. Existing nonce, identity, route and IPv4/port serialization remains
  unchanged for the supported 16/32-byte profiles. Retaining the generic
  compatibility API is reasonable within this bounded change.

- **Duplicated Code:** `replica_provider_allowed` centrally expresses the
  provider eligibility policy. Its positive predicate is logically equivalent
  to the former rejection condition. Both call sites remain: before accepting
  the stream and after awaiting and validating the request, before any manifest
  is emitted. The refactor preserves the required live authorization recheck.

No new documented-standard violations or actionable baseline smells found in
this supplement. The recovery note records the constrained profile explicitly;
the review-check wrapper adds the affected verification selections.

Inspected `review-maintenance/results.json`: all twelve real recovery checks,
legacy discovery, all-targets with current hooks, formatting and owned-container
cleanup exited zero, with `passed: true`. The legacy log reports two passing
checks. Tests were not rerun during this independent review; unchanged slow
NAT matrices remain excluded under the user's standing instruction.

**Final Standards result: 0 hard violations; 0 remaining optional findings.**

## Spec

Reviewed `c6134e27098981fa9754b8265d47c91d083b729e...95b536d0f5284a86fb145a66d8333800dc7ebf17`, using the complete MIOPUNCH-46 and parent MIOPUNCH-14 snapshots, current CONTEXT, ADRs 0004/0005/0007/0008, the validated discovery profile and recorded recovery integration choices. Uncommitted user documents and `.cargo/config.toml` are excluded.

**Findings: 0.** No concrete missing/partial requirement, unrequested behavior, or incorrectly implemented requirement found within this ticket's declared native Linux MQTT/IPv4 profile.

The requirement “A 不知 C 身份、B 全程离线，A 自动发现 C 并经真实数据路径取得需要的状态，进入正常范围” is exercised through `resume_network`, independent persisted identities, actual MQTT discovery, the original authenticated connection's UDP path, reused LogSync and subsequent authorized business communication. The reopened-startup check verifies C's authorization survives A's store reopen.

“只把候选当线索；保留资源资格、阶段准入和真实身份核对” is reflected in Lookup+Signaling resource qualification, origin-tag validation, exact TLS EndpointId, session-bound admission proof and source membership checks before any history manifest. The fresh identity's old secret grants Bootstrap but neither history nor Member access. Wrong network/generation, missing material, false candidates and known revocations have dedicated checks.

“候选失效、入口部分失败、重复线索和缺依赖均可恢复或明确等待” is covered by retry on the same endpoint, scoped contact backoff, duplicate suppression and `WaitingForDependencies` retaining saved/applied source progress. `Ready` requires durable source completion and current authorization, with no global freshness claim.

Reviewed retained acceptance/regression receipts and relevant test assertions; no tests rerun during this review. The earlier resource regression is retained and its corrected check passes. Evidence does not establish broader NAT, relay-only unknown recovery, automatic IRC discovery or other-platform support, and the implementation documentation does not claim them.

### Independent final supplement

Reviewed only `95b536d0..3eb04819` in `src/mqtt_discovery.rs`, `src/network_recovery.rs`, `src/offline_replica.rs`, `scripts/check_network_recovery.py` and the recovery note, against the same complete ticket/parent specifications and accepted decisions as [the original review](spec-review.md).

**Remaining Spec findings: 0.** No missing/partial requirement, scope expansion or incorrect required behavior introduced by this maintenance.

The requirement “只把候选当线索；保留资源资格、阶段准入和真实身份核对” remains satisfied: the provider helper preserves the same predicate at both original authorization checkpoints. Recovery still requires Bootstrap admission, an IP data path and current membership before providing history; neither checkpoint was removed or weakened.

“将已验证的候选发现 profile 接入同一恢复流程与地址簿” remains unchanged. Legacy routes retain `MDR1` with sixteen bytes; configured origin-qualified routes retain `MDR2` with thirty-two bytes. `TaggedCandidate` names the existing type. The explicit private profile centralizes the format selection and rejects unsupported responder widths before broker access. Private discovery still decodes unsupported widths as invalid; its production callers use only the two supported widths. The note's broad early-rejection wording should be read as responder validation, not a new discovery capability.

The additional runner option only selects existing acceptance, legacy-profile, typecheck and formatting checks. It introduces no runtime behavior or product scope.

Reviewed `review-maintenance/results.json`: all twelve real acceptance checks pass, alongside both legacy bootstrap tests, all-targets/current hooks and formatting. The startup log explicitly confirms durable authorization after reopen. No successful checks were repeated during this review. Native Linux, owned MQTT and IPv4 UDP remain the qualification boundary; broader NAT/IRC/platform behavior remains unclaimed.

The Spec precision note was accepted in the integration note and evidence index:
early unsupported-width rejection is explicitly scoped to public responder setup;
private query call sites use only the supported profiles. This final wording change
is documentation only and adds no runtime coverage claim.

Standards: 0 hard violations, both initial optional findings closed, 0 remaining;
Spec: 0 initial and final findings. No unresolved issue within either axis.
