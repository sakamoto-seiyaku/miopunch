# Revocation, causal correction and rejoining

MIOPUNCH-34 uses the existing signed causal verifier, durable management history,
and one shared AdmissionGate. Baseline: `91cc520cbae87c5d8f02c7c754eab95af0eae375`.

The gate indexes actual QUIC Connection lifetimes using stable IDs, weak
connection handles and closure futures. A protocol actor finishing never removes
a still-live connection from this index. Effective view refreshes from local
management, real LogSync application and replay reach this same owner. New
removal closes every indexed session involving that peer, including restricted
requests; local departure closes the node's indexed sessions too. Bootstrap and
member scopes remain invalid while either endpoint is locally known removed.
No admission-secret rotation is required.

A new restricted admission-request connection is the deliberate exception for
fresh authorization after valid removal. It grants neither membership nor
management history. The manual/precode handler still authenticates the TLS node,
checks the pinned invitation and queries the current causal authorization of the
specific durable result. An old request or redemption remains rejected, even
after a different grant restores the same identity. Recovery, bootstrap, normal
LogSync, Gossip, Discovery and business entry stay rejected for known removed
peers. Existing restricted connections are closed when removal becomes effective;
only a newly established restricted request may proceed.

`begin_rejoin` accepts an invitation for the same network after loss of local
membership, commits a fresh attempt in the private invitation file by fsync and
atomic replacement, and preserves node identity, anchor, secret and full signed
history. It does not approve the applicant. Ordinary reopen uses the committed
attempt. Creator recovery material remains distinct and verified even if the
creator later creates a rejoin binding. A normal member with valid membership
cannot use this operation to replace its pending completion request; alternate
contact completion is the separate MIOPUNCH-35 slice.

Voluntary departure uses the public management RemoveMember action targeting
self. The same causal prestate/signature rules permit ordinary self-removal and
require current genesis elevation for administrator self-removal. Disconnection
only closes endpoints and retains durable membership/administrator roles.

Both actual arrival orders of B removing real member T and the root concurrently
demoting B reuse StrongRemove and the complete management graph. Removal-first
closes the actual original business Connection. Correction permits a new actual
Connection, while original handles remain closed. The filtered removal is retained
as verifiable history, and reopen preserves the corrected membership. This is
state correction, not old approval undoing a valid removal.

Evidence and rerunnable checks: [mio34-revocation](../evidence/mio34-revocation/README.md).
Native checks use Linux loopback, actual Ed25519, SQLite and iroh TLS/QUIC.
Browser validation is compilation, not browser runtime revocation. No physical
power-loss, secret rotation, unknown-member discovery, member relay, full UI,
service ACL, TUN or new transport/congestion feature is claimed.

The restricted-request split exposed a final-stage recovery race during review of
handshake interleaving. A raw real TLS peer waits for the gated client's stage
byte, then the gate receives an effective removal before the peer returns its
byte. The initial implementation incorrectly returned an admitted recovery
Connection. Session.valid now rechecks removal for GenesisRecovery at final
registration, while only AdmissionRequest gets the rejoin exception. The
regression uses actual Endpoint hooks and stream synchronization, not sleeps or
mocked handshakes. Initial direct-rustc dependency-artifact mismatches and the
first lazy-QUIC-stream fixture timeout are retained separately from the genuine
runtime RED and post-fix GREEN. Generation1 finished all five checks on unchanged
pre-fix source; it is historical evidence, not the final fix's validation.

Final verification: source-fixed generation2's complete original raw log has
**294 passed, 0 failed, 4 existing ignored**, 34 successful summary blocks,
no failed/error/skipped marker, and real MQTT **98 publishes**. Its Cargo terminal
exit is **unknown/null** because the wrapper disappeared after the last doctest
footer; no terminal zero is inferred. The runner no longer existed and left only
its exact owned broker. Only the missing WASM/default/fault/format checks were
continued, each with actual exit0. That broker and its two anonymous volumes were
removed; no unrelated resource was changed. The full suite was not rerun merely
to recover the lost exit. The final2 receipt records this boundary honestly.

Generation2's pre-start SHA manifest did not survive: the original wrapper kept
it only in memory until returning from Cargo. Current16source/harness SHA values
were first persisted after wrapper loss and stayed unchanged through the resumed
checks. All input file mtimes precede actual broker creation/start; six original
compiled test binary SHA/mtime/dep-info bindings and actual test-list presence
are recorded. These establish available provenance without claiming an observed
continuous pre-start SHA audit during the original suite. Independent final reviews inspect
these limits. Earlier post-fix affected gate/revocation checks
passed23/0, the earlier wide affected short passed67/0, and creator rejoin passed1/0.

Two independent fresh GPT-6.1 Sol HIGH final reviews completed: Standards
**0 required, 1 optional P3 fixture-naming heuristic**; Spec **0 required**.
Both reviewed the same38owned staged paths and independently verified all25
lossless logs, current16source hashes, six compiled test bindings and the
truthful final2 exit/provenance limits. No runtime or harness changes followed.
The final40path commit adds their verbatim reports and observed prose only.
