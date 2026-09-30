# Designated precode redemption and quota continuity

MIOPUNCH-33 implements the [assigned invitation behavior](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/c85db372-e634-4efb-aa6b-8f1ff904a051)
against parent MIOPUNCH-14, ADR 0002, and the existing manual-admission and daily-management boundaries.
The fixed implementation baseline is `b8f40efc9e7e4d2125cfc504e8724c06b53d7a9d`.
This note records the concrete integration, without changing accepted product behavior.

`NativeNetwork::issue_precode` issues a code for this node after verified administrator recovery.
Every code, including time-only codes, pins the canonical network anchor and designated contact.
Its 128-byte trusted-channel encoding contains that public invitation and a random opaque bearer token;
it contains neither the network admission secret nor a node/root private key and has no Debug representation.
Expiry is an absolute Unix deadline, with an optional positive finite usage limit.
Transport hints still use existing endpoint configuration. The designated node retains policy and consumption locally;
other administrators have no redemption or single-code revocation authority over that registry.
There is no automatic offline takeover or distributed quota.

The restricted real iroh TLS admission connection authenticates the applicant before redemption.
The claimed applicant must equal the actual peer, and its invitation must match this node and network.
A persisted joining attempt plus code deterministically identifies its redemption, using BLAKE3 only to derive
an identifier namespace. Changing code creates a distinct attempt; it is not a new authentication protocol.
Identity remains a separate key in the result relation, so sharing a code/request ID cannot impersonate an applicant.

One SQLite transaction conditionally increments consumption and saves the code/applicant/request relation,
the signed authorization and the shared `ApprovedAdmission` pending-delivery result.
The node locally serializes commands; a conditional database update also enforces the last-use limit.
Stored consumption must agree with its complete redemption ledger and approved delivery references;
it is never inferred from the current member list. Detected inconsistent evidence rejects new consumption.
Both sides of the commit have actual process-exit fault hooks only in the existing `replica-test-hooks` build.
Recovery applies the original committed signed record before any membership/dedup decision.

The implementation shares manual admission's authoring and two-material delivery path.
Existing valid membership reuses its signed grant; it does not blindly add the member again.
The same code, authenticated identity and attempt retrieve the same durable grant and delivery progress after restart,
without another decrement. Expiry, exhaustion and single-code revocation constrain new redemption;
valid prior membership/results remain independent. Delivery still requires the original authorization to remain valid
and this delivery node to hold current administrator authority. Authorization and the current admission secret are
verified and saved separately; sender acknowledgements never substitute for the joining node's durable completion.

Each code stores the actual signed administrator authorization origin, not a current role boolean.
The shared causal graph checks Create/Add/Promote Manage grants against effective Demote/Remove history.
A loss outside the grant's ancestry invalidates that episode even when a later root grant restores Manage.
Earlier losses already in a fresh grant's ancestry do not invalidate the fresh episode.
Selected StrongRemove corrections and quarantined invalid operations retain their existing meanings.
Delayed signed loss/regrant history therefore invalidates old codes even when the receiving node's role was Manage
before and after the history exchange. Previously granted valid ordinary membership is retained.

## Supported backup boundary

`create_backup` consumes/stops the node, explicitly truncates/checkpoints its SQLite WAL, closes storage,
and atomically publishes a coherent private backup containing only the node's durable materials.
Bound endpoints must be closed, and callers must avoid concurrently opening the same data directory.
A creator backup preserves its independent root private key **and** canonical Create hash;
an ordinary joined node does not gain that credential. Trusted backup storage remains the caller's responsibility.

`restore_backup` restores a trusted supported backup into a new directory, validates its canonical signed state,
and durably stops every restored code before publishing/opening the restored node.
The explicit restore event identifies lost continuity; a boolean rolled back with the counter is not its proof.
A saved counter of zero after restoring a coherent pre-consumption backup does not authorize reusing the old quota.
Newly issued codes start an independent complete ledger. Stopped codes remain stopped across normal reopening.
Existing committed results in a restored backup can still be delivered while their signed authorization is valid.
An old backup cannot restore results/history it never contained.

This interface deliberately does not claim to detect arbitrary undisclosed consistent whole-disk rollback,
malicious database rewriting, or concurrent use of duplicated node directories.
Ordinary reopen preserves durable counters/results; supported restoration conservatively stops old codes.
An actual closed-database counter corruption fixture, distinct from the backup test, verifies ledger disagreement
stops the old code and permits fresh issuance.

## Evidence and limits

The first real check redeems a single-use code, interrupts material delivery, reopens both nodes,
and retries the same identity/attempt. The priority check establishes two independent real TLS connections
before concurrently contesting the last use; exactly one completes and consumes one grant/use.
Further individual checks cover expiry, time-only revocation, same-network designated ownership/offline refusal,
TLS applicant spoofing, code-scoped fresh completion, signed authority loss/regrant, delayed LogSync authority history,
actual supported pre-consumption restore, persisted restore stops, counter damage, and both commit-boundary process exits.
The regrant fixture observes known-removal connection rejection, then uses a separately signed public management Add;
old results remain invalid while fresh-code completion reuses that valid grant.
The initial regrant fixture incorrectly assumed the removed identity could reach the handler; both genuine failure logs
are retained. The corrected fixture observes the connection boundary. MIOPUNCH-34's rejoining lifecycle policy and
MIOPUNCH-35's changed-contact completion remain separate scopes.

Reproduction, actual final checks, source hashes and lossless original logs are in
[the evidence directory](../evidence/mio33-precode/README.md).
The environment is native Linux loopback with real Ed25519, SQLite, iroh TLS/QUIC, admission and LogSync.
The final workspace check uses its own real MQTT broker to cover the affected shared delivery path.
Browser validation is compilation only. No physical power-loss, new NAT success rate, broader platform runtime,
formal UI, rotation, full root-recovery lifecycle, migration, service ACL or TUN capability is claimed.

The sole final source-fixed generation completed **288 passed, 0 failed, 4 existing ignored**,
with real MQTT capture of 97 publishes and all five checks exiting zero. The affected short
regression was 47/0. Twelve source/harness hashes remained unchanged through final verification
and both independent reviews. Standards found 0 required and 1 optional timeout-maintenance
observation; Spec found 0 required or optional findings. No source/harness changed afterward.
