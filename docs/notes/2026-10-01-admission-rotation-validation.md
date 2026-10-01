# Admission-secret rotation

MIOPUNCH-37 implements the rotation behavior in the
[parent specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
The fixed implementation baseline is `a6e384ca9d19595721a1b373ebde47feb21efade`.
The domain rules remain those in CONTEXT and ADRs 0004, 0007 and 0008.

## Selected integration

`NativeNetwork::begin_admission_rotation`, `distribute_admission_rotation`,
`receive_admission_rotation`, `finish_admission_rotation`, and
`rotation_progress` expose the flow through the existing durable node.
`remove_member_and_rotate` makes explicit removal effective before starting or
resuming rotation. The ordinary management removal interface stays independent.

The current device/session must have a valid genesis elevation to begin and to
commit the switch. A root-signed proposal pins the canonical network, random
rotation identifier, old/new generation, initiator identity, retained snapshot
and a one-way tag of the replacement credential. Every current device member
counts, including an already authorized member still awaiting initial material.
The virtual root author in ADR 0007 does not count as an online device.

Private staged files are fsynced before an Ed25519 acknowledgement of the exact
proposal is made. Each acknowledgement identifies its node, binds the signed
proposal, and is verified against the actual TLS peer at receipt. A retained original
provider may forward that exact signed acknowledgement over its own proved
session: its own acknowledgement must still identify the actual TLS peer, and
every inner signature independently verifies the named node and exact proposal.
The initiator commits
it to SQLite FULL before counting it. A lost acknowledgement can be resent for
the same proposal; a recipient's local durable receipt does not make the sender
assume delivery succeeded. A root-signed completion carries the retained
identities and their verified acknowledgements. The initiator rechecks actual
current membership and genesis authority at the commit boundary.

The signed completion is the durable switch decision. Publication of the
active private material and its database credential pin follows that decision;
reopen repairs an interrupted publication before exposing an endpoint. Later
admission may advance the active credential beyond an old completed rotation;
historical completion cannot roll the current credential backward. Node keys,
network root and canonical Create stay unchanged. Root private keys and raw
admission secrets never appear in the signed control statements or progress.

A fresh restricted rotation connection negotiates one active/staged generation
and performs the selected SCRAM-SHA-256-PLUS proof once on that TLS session.
Staged secrets qualify only the rotation protocol; ordinary bootstrap, LogSync
and business still use the active generation. Switching closes old proved
sessions. Completion reaches a staged recipient on a fresh new-generation
connection, without repeating authentication on an existing TLS session.

An already approved retained node can lack the old secret. Its original
provider's current durable pending grant permits the existing secure material
delivery exception for the replacement only. Ordinary material resumption is
still paused. Ordinary approval and root recovery keep separate evidence: a
pending root-recovery recipient must additionally present the original root
proof bound to the new TLS exporter, canonical Create and both node identities.
The existing root SessionGuard ends that authentication instance. An ordinary
approval, a saved role flag or TLS node identity cannot substitute for it.

## Admission/material entry inventory

All processing/delivery checks read the same durable rotation pause. The
receiver saves the non-secret signed pause before waiting for replacement
bytes, so a crash before secret commit leaves a retryable unacknowledged plan.
Replacement distribution itself continues during that pause.

| Current entry | Pause check |
| --- | --- |
| Manual request serving, new manual approval | Before request/approval processing |
| Approved manual material resumption | Before history and actual secret release |
| Precode new redemption and original result resumption | Shared serving path and redemption check |
| Manual/precode request and completion on an informed recipient | Before processing and private secret persistence |
| Direct Add/new administrator membership authoring | Before creating new membership |
| Effective-removal rejoin and new-code completion preparation | Before publishing the new admission binding |
| Current management restoration serving/receiving (MIOPUNCH-36) | Before processing, history and secret release/persistence |
| Earlier narrow genesis-recovery delivery | Before protected history/current-secret release |
| Retained replacement distribution | Permitted; exact signed rotation and current grant/member checks apply |

Invitation/code issuance and inactive local `join`/`recover` preparation do not
process admission or release the network secret. Read-only progress/history
queries, existing ordinary communication and signed management-history receipt
continue. Pause reflects verified information available to the node; this does
not promise an instantaneous global update at an unreachable device.

## Verification and limits

The public test seam is `tests/admission_rotation.rs`. Core signatures, SQLite,
iroh TLS/QUIC, persistent node directories and process exits are real. Fault
builds expose commit checkpoints; they do not replace authentication or storage.
Final evidence and exact executable commands will be recorded in
`docs/evidence/mio37-rotation/`.

The first real check used two external retained members plus the initiator,
then reopened all three and used a fresh new-generation connection. The
priority failure used a receiver process exit after receipt and before storage;
the initiator waited with only its own acknowledgement and kept the member.
The historical first priority result predated persistence of the pause before
secret receipt; the later source requires the reopened unacknowledged plan too.
Genuine compile errors and failing public tests remain separate evidence.

The explicit user policy permits omitting unchanged previously verified slow
NAT tests. This ticket changes admission gate handling and credential lifecycle;
meaningful affected gate/NAT credential-invalidation checks are required, while
unchanged NAT algorithms, probability matrices, scheduler/listener experiments
and frozen MIOPUNCH-60–63 evidence remain the prior basis. Exact exclusions,
source comparisons and skip-name collision audit belong in final evidence.

Native runtime verification is Linux loopback. WASM is compilation coverage;
physical power loss, broader platform runtime and a security audit are not
claimed. Temporary elevation timing uses the existing monotonic-window
interface. UI, migration, service ACL, TUN, KCP, Brutal/custom congestion control
and new cryptographic policy remain outside this slice.

## Existing admission across generations

A completed historical root recovery can later be effectively removed and
ordinarily re-admitted after more than one rotation. Its old private admission
file must not make a new verified grant look Complete before current-generation
material arrives. A real public lifecycle test first exposed that stale-success
case. The ordinary provider now sends its current generation before history;
the recipient saves that expectation after verifying history and before
applying the grant, suspends obsolete proof material, and binds its opaque
delivery to the advertised generation. Reopen enforces the saved expectation.
Private active-material replacement is atomic and only advances generations.
The existing old signed root grant remains invalid after ordinary rejoin.

Initializer crash checks also exposed both sides of the initial durable-plan
publication. A staged private file left before a proposal is published is reused
for the still-unused next generation. A published pending proposal resumes the
same rotation identifier and repairs its initiator's durable self-acknowledgement.
Neither path overwrites a different published pending proposal.

## First final generation (before independent Spec corrections)

The final runner completed all 13 actual commands with exit 0, including 84
selected passing tests, zero failures and zero ignored tests. The complete
rotation target passed 28 entries (27 behavior cases and one no-environment
worker), including 13 observed child exits 87 at genuine persistence boundaries.
Default/fault typechecks, WASM compilation and formatting passed. An isolated
real MQTT broker captured 95 ciphertext publications; its container and both
anonymous volumes were removed and independently inspected as absent.
The full source/runner prestart SHA manifest and archived source bytes remained
unchanged through the checks. See the [evidence index](../evidence/mio37-rotation/README.md)
for exact commands, executable SHAs, actual exits, lossless logs, historical
RED generations and the unchanged-NAT exclusion basis.

## Independent review corrections

The first independent Standards review found no required correction and one
optional possible duplicated private-publication helper. The first independent
Spec review found two concrete recovery gaps. Both reports and their prior
final1 generation remain preserved in the evidence folder.

A member may durably acknowledge generation N while missing its signed
completion; an initiator can meanwhile start N+1. Distribution now carries
only the exact root-signed predecessor completion needed by that member. The
receiver matches it to its own staged proposal, verifies the completion and
all node signatures, durably commits and activates it, then accepts the new
proposal. A later proposal never substitutes for the predecessor decision.
A newly approved secretless recipient with no predecessor plan still uses the
original pending-delivery proof profile and directly stages the current
replacement; it is not required to possess a secret it never received.

A different elevated device can initiate rotation while a retained pending
recipient pins its original provider. A currently retained provider can now
forward the exact verified root proposal and private staged replacement via
that original narrow profile, collect the recipient's genuine durable signed
receipt, and forward it back on its own proved rotation session. The initiator
verifies both the TLS peer's own acknowledgement and every forwarded signature,
and commits each receipt before counting it. The original grant/contact and
fresh original root exporter proof remain mandatory for pending root recovery.

Public RED tests reproduced both original failures. Public GREEN tests then
covered missing predecessor completion with both nodes reopened and generation
2 LogSync; ordinary original-provider relay with all three nodes reopened,
actual forwarded-signature tampering rejected without counting, and the same
original grant; and pending-root relay with a genuine node-key signature denied
before history, a fresh original root proof, and stable original recovery
request/grant after all nodes reopen. A fixture-only queue assertion failure is
retained separately: the additional normally admitted initiator legitimately
had an existing request; the corrected check preserves that original queue.

The final2 runner selects the full 31 rotation entries plus WASM/default/fault
compilation and formatting. [final2-reuse-basis.json](../evidence/mio37-rotation/final2-reuse-basis.json)
records the exact final1 denominator: 54 of 57 source/harness inputs unchanged,
only the rotation module, rotation tests and runner selection changed. The
prior raw-wire/admission/NAT/MQTT checks are explicitly historical evidence for
unchanged paths; no new-source 84-pass or fresh NAT/MQTT result is inferred.

## Final generation guard and current result

The stronger final2 root-provider case began after a completed standalone
generation1, then prepared a new root recipient without that secret for a
different initiator's generation2. It revealed that the catch-up guard treated
the recipient's already-staged generation2 as predecessor1. The full31 run
completed with actual30PASS/1FAIL/Cargo101 and wrapper1; the four following
compilation checks were unstarted. All evidence is preserved.

The correction is one runtime predicate: catch up only when the pending local
proposal's generation equals the incoming proposal's previous generation.
Receiving that same staged proposal's exact signed completion still validates
all existing root/member/ACK checks before activation; it need not invent a
secret or proposal the newly admitted device never possessed. An isolated
actual root-generation2 GREEN passed, followed by final3's source-fixed three
recovery cases, all six command exits0 and WASM/default/fault/fmt. Current57
source/runner inputs match the archived prestart snapshot. See the final3 reuse
basis and one-hunk patch; 55 of57 final2 inputs remain byte-identical. Neither
historical84PASS nor the earlier full31 result is a latest-source success claim.

## Final independent reviews

The separate final Standards review found zero required corrections and one
optional possible duplicated private-publication helper. The separate Spec
review found zero required/optional corrections and closed both initial P2s.
Both independently audited current source/archive/executable hashes, real
command exits, single-hunk recovery predicate, bounded affected revalidation,
lossless historical evidence, protected files and unchanged-NAT exclusions.
Reports are preserved in the evidence directory; the optional helper suggestion
did not require an unrelated refactor.
