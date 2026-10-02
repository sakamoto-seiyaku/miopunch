# MIOPUNCH-48 Standards review

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
