# MIOPUNCH-36 durable original-network management recovery

Assigned ticket: [MIOPUNCH-36](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/322a614c-4d6f-4fac-973b-ef2c77912e18).
Fixed baseline: `f691d2b4c2c0f8188d9e9418ef691c21978a806c`.
Behavior and integration boundaries are in
[the validation note](../../notes/2026-10-01-management-recovery-validation.md).

Reproduce the complete combination with an isolated real MQTT broker:

```sh
rtk proxy python3 scripts/check_management_recovery.py --output /tmp/mio36-new-run
```

Use `--affected-lifecycle` to reproduce the final P2 repair's narrower generation:
ten recovery/node/store/auth/gate/admission/rejoin targets with a real broker,
WASM/default/fault/fmt, excluding four unchanged NAT and unrelated prior modules.

The output directory must be new. The runner persists54 native/browser source,
test, Cargo, broker-config and harness hashes before broker creation, then saves
each command's actual exit separately at its boundary. It cleans only its exact
owned broker/anonymous volumes and records the actual inspect exits and outputs;
absence requires the object kind, owned ID and a case-insensitive Docker missing
object response. The source comparison is made at command boundaries, without
claiming continuous observation at every instant.

Following the user's explicit unchanged-NAT test policy, the current runner
defaults to45 exact-name filters across `legacy_multi_socket`,
`legacy_nat_runtime`, `legacy_nat_scheduler` and `udp_observation` (44 passed
cases and one existing ignored CPU entry in this actual run). Their NAT,
packet/connection and transport implementations are unchanged by MIOPUNCH-36;
the native/gate changes add only the separate recovery lifecycle/ALPN, while
the shared admission boundary still has its actual affected regressions.
Use `--include-nat` when those implementations or an affecting shared path
change. The scope basis, complete45-name inventory and both exact command
plans are in [`postrun-nat-policy.json`](final1/postrun-nat-policy.json).
The frozen [60](../mio60-formal-nat/README.md),
[61](../mio61-legacy-multi-socket/README.md),
[62](../mio62-legacy-selector/README.md) and
[63](../mio63-legacy-scheduler/README.md) evidence is retained; no probability
or profiling batch was repeated.

## First check and genuine failures

The first public entry test compiled RED101 because the formal node recovery
API/module did not exist (`first-red`). Its first real GREEN (`first-green`)
restored the same network from the original separate root-key/Create backup,
with all old administrator endpoints/stores offline and only an ordinary member
online. The fresh node started with0 records and no secret, retained its distinct
identity, signed its own root grant, saved the current secret, used daily
administration and reopened successfully without clearing the old administrator.

The priority `ordinary_contact_requires_root_proof_but_no_secret_history_or_old_approval`
passed1/0 (`priority-proof-gate`): an authenticated but unproved peer gets no first
history byte and no authorization side effect; the legitimate new node lacking
secret/history completes from that same ordinary contact without an ordinary
request, precode or offline administrator.

`authorization-crash-red` is a genuine missing-checkpoint RED: the child completed
instead of exiting86. Connecting the observer reached real exit86, then
`authorization-crash-green` exposed `Store(Auth(InvalidLog))` during resumed
preflight. `authorization-crash-probe` retained a targeted diagnostic showing
the original persisted grant ID and exact already-applied bytes. The fix verifies
those saved bytes and their current authorization instead of treating them as
new authoring; new records keep the log-continuity preflight. The diagnostic
instrumentation was removed. `authorization-crash-fixed` passed1/0 with real
exit86, both-store reopen, new TLS proof and the same signed grant. No failed
receipt or source generation is relabelled as a successful run.

## Actual process-exit matrix

Each point was added and executed separately. All16 tests reached actual
process exit86, reopened both independent SQLite stores, proved the root on a
new real TLS session, retained the stable attempt and any existing signed grant,
and completed with exactly3 applied records and no ordinary approval. Current
grant validity is checked on every retry. The original administrator remains
Manage and the provider remains Read.

| Boundary | Crashing node | Reopened receiver observation |
| --- | --- | --- |
| Signed root proof saved | Contact | Awaiting proof; no history or secret |
| Full signed history verified, before save | Recovering | Awaiting proof; no history or secret |
| Verified history saved/applied | Recovering | Proof verified; no grant or secret |
| Own grant/result transaction committed, before apply | Recovering | Same grant applied on reopen; pending secret |
| Relay grant/result transaction committed, before apply | Contact | Same local grant; pending secret |
| Own authorization saved/applied | Recovering | Pending secret |
| Grant acknowledgement sent | Contact | Pending secret |
| AUTH sent | Recovering | Pending secret |
| Current secret prepared | Contact | Pending secret |
| Secret sent | Contact | Pending or complete, according to actual receipt/save |
| Secret received, before save | Recovering | Pending secret |
| Private secret file published, before database pin | Recovering | Pending secret; retry must pin it |
| Secret saved/pinned | Recovering | Locally complete |
| SAVE sent | Recovering | Locally complete |
| SAVE received, before provider completion commit | Contact | Locally complete |
| Provider completion committed | Contact | Locally complete |

These establish process-exit/reopen behavior, without a physical power-cut claim.

## Signed security and affected regressions

Ten separately executed real signed/TLS cases passed1/0 each: wrong root,
same-root other signed Create hash, substituted proof target, substituted
grant target, old root log ID, old graph tips, saved-proof replay on a new TLS
session, and an actually valid same-root alternate complete history rejected by
the original canonical pin. A verified demotion propagated by actual LogSync
then survives an old signed snapshot and rejects its old saved recovery result;
that result cannot become a new grant. A completed node/relay reopen and retry
the same grant without another Add.

Further individual cases passed1/0: both old administrators offline and retained,
default formal endpoint support plus explicit genesis elevation after recovery;
another device's TLS handle rejected; another legitimate member cannot replace
the saved contact; genuine loss of the only online provider's private secret
fails closed with no history/success or forced ordinary approval.

The regular default and fault-build workspace/tests typechecks exited0.
The affected signed-auth/store/native/gate/daily/old-proof regression passed
46/0/0 and exited0 with its pinned inputs unchanged. Per-phase source manifests,
raw logs and actual command-exit receipts retain these observations and the
earlier failures. Each short command is also reproducible directly with the
`cargo test` arguments in its exit receipt.

## Final verification

The original pre-review source-fixed full run passed **347/0/4 existing ignored**, with
all33 new recovery cases passing and **95 actual MQTT publishes**. Cargo's actual
exit0 was saved after975.086s; WASM/default/fault/fmt also each exited0.
The overall actual wrapper exited0. Its owned broker and both anonymous volumes
were removed with cleanup exit0 and independently inspected absent with their
real kind/ID/error outputs. The pre-start54 inputs stayed unchanged through all
five checks. All manifests, five individual exits and the actual wrapper exit
are in [`final1/`](final1/). The raw result sum347 includes one nested
multisocket child pass; the top-level harnesses report346 passes. The original
aggregate receipt is retained unchanged.

The user introduced the NAT exclusion policy while this original run was
already in flight. Its NAT groups naturally completed before a safe supported
filter could save remaining NAT work; Cargo cannot dynamically change test
filters. No whole round or successful module was restarted. Existing logs give
multisocket28.64s, legacy-runtime59.17s and UDP-observation28.76s, versus recovery
286.20s, old delivery169.22s and replica129.41s. The main cost here is process
interruption/recovery, rather than the approximately116.57s of these NAT groups.
No extra profiling execution was needed.

**Only the runner's future selection/report label changed after completed
checks**, to apply the explicit user policy. The exact actual-run runner is
preserved in [`runner-during-checks.py`](final1/runner-during-checks.py), matching
its original pre-start SHA. All53 other inputs stayed unchanged. The current54
hashes are separately pinned in
[`postrun-source-sha256.json`](final1/postrun-source-sha256.json).
Offline command selection passed:45 unique intended filters, no substring
matches in other observed test names, and `--include-nat` generates the exact
original Cargo command. The first read-only inventory probe counted a nested
child name twice; its actual exit1 and deduplication correction are retained in
[`nat-selection-first-probe.json`](final1/nat-selection-first-probe.json).
That postrun filtered workspace runner version was never executed; no filtered
whole-workspace suite was run in this delivery. The original completed run
excluded no tests. Its original exits,
receipt and provenance are not rewritten or attributed to the newer runner.

All raw logs are retained losslessly in gzip with original/compressed/roundtrip
SHA256 and byte counts in `lossless-log-manifest.json`. Independent reviews and
subsequent repair retain their actual generations; earlier failures stay intact.

## Independent Spec repair and final current generation

The initial147-path Standards review found0 required/optional findings.
Spec independently found one required P2: the unconditional root-binding guard
prevented a completed recovered device from ordinary rejoin after effective
removal. Both verbatim initial reports remain in `standards-review-initial.md`
and `spec-review-initial.md`; the original full347 belongs to that earlier
source, rather than the final repaired implementation.

`completed-root-rejoin-red` reproduces P2 through actual root recovery, explicit
new-administrator appointment, actual LogSync, root-signed effective removal,
reopen and rejected fresh rejoin (actual101). The repaired sequence passed1/0
(actual0), retaining identity, canonical anchor, history/materials and the old
grant's invalidity. New ordinary approval creates a different request/grant and
only Read membership. Current membership still derives from signatures; the
historical completed stage only permits preparing that new request.

Two additional actual exits86 cover historical completion sealed before binding
publication, and the fsynced ordinary binding before the memory flow changes.
Reopen sees the safe previous or new ordinary flow respectively. Both resume
the same ordinary request across another reopen, require actual fresh approval,
retain the invalid old result, and finish durably as Read. The first publication
harness compile failed101 because its request assertion moved a Rust value;
the genuine compile failure and corrected actual86 GREEN are retained separately.

The final repaired generation passed **116/0/0**, including all36 recovery
tests and both new actual86 switch boundaries, with **85 actual MQTT publishes**.
Cargo exited0 after441.456s; WASM/default/fault/fmt and actual wrapper79579 each
exited0. All54 prestart inputs remained identical through the five checks and
match the final current runtime/test/harness files. Its exact owned broker/two
volumes were removed and independently inspected absent. Precise ten affected
targets, source manifests, actual exits and resource receipts are in
[`final2/`](final2/). This execution exists because of the genuine P2 source fix;
it does not repeat the original complete workspace or the four unchanged NAT
modules/probability/profiling batches. The shared admission-gate regressions
remain relevant to the reopened node's actual authorization boundary.
All54 logs across both generations and real failures are retained losslessly.
The two independent latest179-path rereviews found **0 required findings**.
[Spec](spec-review.md) reports0 optional findings and closes the original P2;
[Standards](standards-review.md) reports one optional P3 fixture-duplication
suggestion. That maintenance suggestion does not require another source change.
The final54 runtime/test/harness inputs remain identical to final2.

No runtime/test/harness source is edited during this final generation.
Native Linux loopback, real signatures, SQLite, TLS/QUIC and LogSync are the
runtime environment; browser verification is WASM compilation. No secret
rotation, migration, automatic administrator revocation, UI, service ACL, TUN,
physical power-loss, new NAT probability or broader platform runtime is claimed.
Prior frozen NAT evidence, prior full-suite receipts and original dirty documents
remain unchanged.
