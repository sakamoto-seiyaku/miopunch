# MIOPUNCH-92 restricted endpoint evidence

The additive interface is implemented in commit `e878d95b`, against the approved
M91 source baseline `c08e9b4b`. See the
[API and lifecycle note](../../notes/2026-10-18-restricted-endpoint-api.md).
The ordinary and restricted interfaces share the original connection, socket,
TLS, QUIC, path actor and transport implementations.

This slice provides usable restricted Builder, Incoming, Accepting, IntoFuture,
custom `accept_with`, control-only delivery and Router paths. Real authenticated
control and fresh path confirmation precede the original handshake hook and
ordinary Connection handoff. The original SDK remains on its transitional hook
path; M93/M94 migrate its consumers and M95 restores the remaining original
early-data/hook source contracts. Completing M92 does not complete M91.

## Current verification

[validation.json](validation.json) records the final runtime results, all 16
compiler consumers, current Native/browser SDK results, artifact hashes and
cleanup outcomes. All final runtime selections are nonempty, use frozen current
artifacts, exit normally and pass their source/tool/artifact postconditions.
There are 121 passing runtime tests across 17 targets, including the 19 new
interface/ordinary early-data cases, plus 16 compiler checks. The final selection
combines unchanged passing targets with the corrected admission target below;
it is not a claim that the initial failed batch passed.

The 19 public-interface tests cover control and business handoff ordering,
control-only capability isolation, typed routing, custom TLS acceptance, missing
and rejected decisions, absent and foreign path proofs, legacy hook rejection,
authorization withdrawn during the hook, malformed/oversized control framing,
caller deadlines, cancellation, dropped futures, weak-handle revocation after
handoff, and an actual MQTT-to-UDP change on the original connection. A separate
ordinary-endpoint case in the same target exercises incoming early data and
accepted outgoing 0-RTT resumption.

The remaining selected targets exercise existing admission gates, MQTT upgrade,
unified connections, negotiated context, TCP and physical UDP sockets, member
attempts, IRC framing, RTC qualification/revocation and path selection. A private
loopback Mosquitto broker is supplied to environment-dependent MQTT cases; they
are not counted from their no-broker early-return branch.

The complete current Native/browser SDK story uses fresh synthetic identities,
owned loopback Ergo TLS/WSS, a local HTTP data relay and the pinned current WASM
package. It completes an awaiting-approval request, one actual approval, durable
authorization/secret delivery, persistent identity reopen, and a member
PING/PONG stream. SDK actors, browser, origin, relay and container close normally.
Private test keys and NSS databases were removed after actor absence was proved.
SDK logs/identities remain private; the published SDK result is an explicit
allowlist of outcomes. No public facility or physical-device matrix was run.
The controlled SDK and new restricted fixtures use owned facilities. The legacy
regression batch was not packet-captured; its driver intent field is not proof
that all pre-existing fixture defaults produced zero external packets.

## Original consumer and transition boundary

[original-consumer.rs](original-consumer.rs) is the unchanged pre-edit consumer,
SHA-256 `e59e299a02e620408c28b0398b1e3ec8ab451ce9f9111e37af2f68e61bdcc769`.
It compiles against the fixed official iroh 1.2.0 source and its original lock.
Against the current fork it still reports exactly `E0308` for the incoming
early-data return contract and `E0004` for exhaustive hook outcomes. These are
recorded known failures for M95, not compatibility passes or reasons to rewrite
the original consumer. The ordinary runtime test verifies usable current early
data; it does not establish restoration of the old source contract.

The final regression batch also found two pre-existing admission fixture
failures. Both reproduced against the pre-M92 `c08e9b4b` iroh sources with the
same locked graph: one assumed a default proof timeout below seven seconds,
despite the existing 60-second fallback; one mixed a standalone gate peer with
the current unified request protocol and omitted its required caller deadline.
Commit `34282788` corrects only those fixtures, selecting an explicit short
deadline or paired current protocol/deadline contexts and empty resources.
All 18 admission cases then pass. Production sources are unchanged, and the
actual selected inputs of the already-verified Native/WASM SDK and owned relay
still match byte for byte. Their passing runs do not require repetition for a
test-only correction. Baseline failures and both fixture iterations are retained.

[transitional-callers.json](transitional-callers.json) lists the actual retained
Native, Browser, probe and test callsites. No application consumer was migrated
in this commit. [review.md](review.md) preserves the independent Standards and
Spec reviews of the fixed source diff; neither reported a required correction.

## Reproducibility and preserved failures

[raw-verification.tar.gz](raw-verification.tar.gz) contains per-run receipts,
raw compiler logs, selected dependency manifests, frozen build contexts,
selectors, drivers, compiler diagnostics, and the new interface's raw test logs
and captured test sources. [archive-manifest.json](archive-manifest.json) records
each member's hash and size. Pinned binaries, full selected source copies, private
SDK logs and original baseline sources remain under the recorded local evidence
paths; no Cargo target tree was copied. Builds use the repository's shared
`target/` and cache lease, retain external lock rows, and abort before Cargo when
source, lock, tool or artifact preconditions fail.

The archive preserves the expected missing-interface compile failures and the
clean runtime failures that exposed peer-close cancellation and readiness after
hook revocation. It also retains unsuccessful harness attempts: missing selector
dependencies, a rejected Cargo argument, and a wrong reference to an already
locked rand version. Only local selector package rows were corrected; external
versions, sources, checksums and dependency rows were unchanged.

Two preliminary MQTT runs were forcibly stopped at their diagnostic deadlines.
Their logs showed completed control/path/business work and completed endpoint
close, followed by a fixture waiting for transport destruction while retaining
its own endpoint owners. Dropping those owners before joining transport drivers
made the minimized run finish naturally. Debug instrumentation was then removed;
the complete final target passes without forced termination. These historical
failed runs remain failed and are not relabelled as graceful cleanup.
