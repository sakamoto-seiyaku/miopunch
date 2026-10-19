# M103 two-axis review

Fixed point: `70e5d5522deb558fca88b9ba0087dc0376fd8091`.
Implementation: `7780db8970fb10cdd2158d6d07f9749c8102f482`.

## Standards

0 documented violations; 0 actionable heuristic findings. The reviewer read all
ten committed paths and sampled final build/selector provenance, the 48 current
regression results and sanitized SDK qualification.

## Spec

0 findings. The reviewer checked ordinary unchanged consumers, the actual 17-byte
plaintext RED, private per-transmit frame metadata and final sender check, held
PING/ACK while application permission remains false, 48 current tests, the rebuilt
whole SDK, all 606 current source guard hashes, and historical Noq manifest recovery.

Both reviews were read-only apart from their reports; neither reviewer reran
tests, built artifacts, mutated source/Git/Plane or cleaned resources.

Total findings: Standards 0; Spec 0. No outstanding finding in either axis.

## Standards report

# MIOPUNCH-103 Standards review

Reviewed all ten committed paths in
`70e5d5522deb558fca88b9ba0087dc0376fd8091...7780db8970fb10cdd2158d6d07f9749c8102f482`
(one implementation commit).

**Documented-standard violations: none found.** Private frame metadata preserves
the original `Transmit` fields, sender requirements and socket identity under
ADR 0011. Final sender permission checks retain the explicit admission boundary
of ADR 0010. Unconfigured predicates remain permissive, and the fixed old
consumers retain their bytes, consistent with ADR 0012. The compatibility note
records the local implementation decision and distinguishes measured method
counts from CPU or universal performance claims.

**Heuristic smells: none warrant an actionable finding.** Classification is
recorded where actual frames are encoded, accumulated for the complete transmit,
and retained with the buffered packet. The existing sender loop owns the final
permission check. Transparent test wrappers have a present measurement purpose;
their delegation does not justify removing them under the Middle Man heuristic.
The fixed consumer is included directly in the regression suite.

Sampled evidence supports `AGENTS.md` and `docs/agents/build-cache.md`: shared
target, complete foreign lock-row comparisons, pre-Cargo source/lock/artifact/tool
guards, six qualified final build receipts at the same frozen source digest,
and separately preserved historical Noq selector provenance. Sanitized receipts
record 48 passing regressions, unchanged original consumer runs, rebuilt
Native/Browser SDK admission, durable materials, identity reopen, member PONG
and natural cleanup. The Pending STREAM regression checks actual forbidden
ciphertext retries and plaintext delivery; the subsequent held PING verifies
control traffic remains usable after metadata reset.

Limits: source and sampled-evidence inspection only. No builds, tests, cleanup,
source edits, Git/Plane mutations or delegation. Only this requested report was
written. Private SDK logs/receipts were not reproduced; the 249 unrelated WIP
paths and broader physical/public acceptance remain outside this review.

## Spec report

No Spec findings.

Reviewed the ten committed paths in `70e5d5522deb558fca88b9ba0087dc0376fd8091...7780db8970fb10cdd2158d6d07f9749c8102f482` against MIOPUNCH-103, the relevant parent requirements and ADRs 0010–0012. No missing requirement, scope creep or incorrect implementation was identified. Protected WIP was excluded.

The implementation closes the demonstrated buffered-send gap at the existing sender boundary. Actual encoded Data-space frames determine private permission metadata; coalesced/GSO batches combine it, each new transmit resets it, and the runtime rechecks current permission before every sender attempt. Control/validation exceptions match the existing receive gate. Packet and physical-socket checks remain independent, and the original ordinary sender/Transmit contracts remain intact.

Inspected evidence supports all four acceptance criteria. The valid pre-fix case delivers 17 forbidden plaintext bytes after only the application predicate is revoked, with cleanup completed before failure. The final case delivers no forbidden plaintext and never repolls its ciphertext, while a separately buffered PING still reaches the peer and receives an authenticated ACK. RTC qualification, wrong-owner/probe rejection, retirement, lock-free terminal observation and natural pump cleanup are exercised through existing public interfaces.

Forty-eight outer tests pass, alongside unchanged fixed ordinary Iroh/Noq consumers and the sanitized full current Native/Browser SDK approval, durable materials, identity reopen, member PONG and cleanup evidence. All 606 current source hashes match the runtime guard. The recovered historical Noq selector matches its original declared hash; its expanded selector is not misrepresented as unchanged.

Work measurements correctly report method/callback/poll/datagram/transmit entries and bytes, without CPU-cost or performance-SLA claims. Limits: inspected source, targeted raw outputs, receipts and the sanitized SDK summary without rerunning checks or accessing private SDK logs. The broader public/physical/platform matrix remains separate.
