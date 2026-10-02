# MIOPUNCH-46 Standards review

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
