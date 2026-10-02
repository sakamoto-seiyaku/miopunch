# MIOPUNCH-46 Standards review: maintenance closure

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
