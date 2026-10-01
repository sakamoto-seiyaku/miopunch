# MIOPUNCH-35 interrupted admission completion

Assigned ticket: [MIOPUNCH-35](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/063223d0-62c8-4d83-9420-02896053f26c).
Fixed baseline: `33f4ba24b0fd93cde79bea5cff4993a9634be7bb`.

Reproduce the final source-fixed checks with an isolated real MQTT broker:

```sh
rtk proxy python3 scripts/check_admission_completion.py --output /tmp/mio35-new-run
```

The runner creates a new output directory, saves the pre-start source/harness
SHA manifest before creating its broker, and writes each actual command exit
at the command boundary. It removes only its own broker and anonymous volumes,
then checks their absence. The manifest covers the integration, delivery,
authorization and storage sources, public tests, runner and broker configuration.
The receipt reports the five commands and source equality at their boundaries;
it does not claim to observe every instant between them.

## Public behavior and retained observations

The first actual check is the existing formal manual admission test
`crash_after_only_authorization_never_succeeds_and_same_request_resumes_once`:
real approval, receiver process exit77 after authorization before requesting
secret, original sender endpoint/store close, reopen and same-result delivery.
`first-original-resume-real.log.gz` passed1/0. The preceding
`first-original-resume.log.gz` failed because the sandbox denied actual socket
binding. It is environmental failure evidence, not a missing behavior RED.

`priority-api-red.log.gz` records the new contact API missing at compile time.
`priority-green.log.gz` records real precode approval interrupted before secret,
second administrator learning/verifying that grant through actual authenticated
LogSync, original endpoint/store unavailable, new-code completion after reopen,
stable identity/same grant, one use of the new code and no duplicate Add.
No trusted membership boolean or original-code registry takeover is injected.

The first secret-send checkpoint attempt in `secret-send-checkpoint.log.gz` failed
because its observer had not yet been installed; the child completed instead of
exiting83. This genuine harness failure is retained. The corrected
`secret-send-checkpoint-green.log.gz` reaches actual exit83, restarts, resumes the
same result and keeps usage1/applied2. Each of the other twelve checkpoint logs
adds one actual process exit, then observes the reopened public node and resumes
that same code/result. The test feature reuses the existing checkpoint observer;
normal builds contain none of these injection hooks.

| Boundary | Crashing endpoint | Durable receiver stage on reopen |
| --- | --- | --- |
| Authorization prepared | Sender | Awaiting approval |
| Authorization sent | Sender | Arrival may be awaiting or pending |
| Authorization verified, before ingest | Receiver | Awaiting approval |
| Authorization saved/applied | Receiver | Pending materials |
| AUTH acknowledged/progress saved | Sender | Pending materials |
| Current secret prepared | Sender | Pending materials |
| Secret sent | Sender | Arrival may be pending or complete |
| Secret received, before private save | Receiver | Pending materials |
| Secret file fsynced/published, before credential DB commit | Receiver | Pending materials |
| Secret saved/pinned, before SAVE | Receiver | Complete |
| SAVE sent | Receiver | Complete |
| SAVE received, before sender completion commit | Sender | Complete |
| Sender completion committed, before DONE | Sender | Complete |

Every actual exit83 recovery preserves stable identity, original grant and
request, usage1 and applied2, and ends with both materials durably complete.
Existing full-suite manual/precode tests additionally exercise real approval
commit interruption, both precode quota/grant/delivery commit boundaries,
required-body/storage/application crashes, a real material-file failure,
normal reopen, old-backup stopping, counter damage and authority loss/regrant.
Those are reused evidence, not claimed new checkpoint points.

`contact-security-short.log.gz` passed4/0: wrong canonical network, wrong actual
TLS contact, obsolete delivery handle after contact change, return to original
attempt without another usage, delivery handle identity substitution, rightful
resume, and new admission after effective removal. An alternate administrator
reuses only a valid verified grant; a removed identity gets a new signed Add,
and the old result remains invalid after that regrant.

`signed_old_response_cannot_restore_locally_verified_removal.log.gz` replays a
complete, originally valid signed response on actual TLS/QUIC after the receiver
has verified and saved removal. The receiver's durable current graph rejects
it and retains removal without saving a secret.
`duplicate_save_ack_does_not_consume_twice_or_prove_recipient_persistence.log.gz`
sends real duplicate SAVESAVE bytes from the authenticated peer without calling
the recipient's save operation: sender acknowledgement becomes Complete while
the joining node remains PendingMaterials. The legitimate retry really saves
both materials, consumes no additional use, and adds no duplicate member.
Neither sender send success nor transport/acknowledgement proves receiver save.

## Final verification and limits

Final runtime-source-fixed checks passed **314/0/4 existing ignored**, including
all20 new public delivery cases, with **94 actual MQTT publishes** and no broker
test skip. All five actual commands exited0. The original overall wrapper
exited1: although `docker rm --force --volumes` exited0 and removed the exact
container and both anonymous volumes, its case-sensitive `No such` check
misclassified Docker's lowercase `no such volume` as remaining resources.
The original [`final1/receipt.json`](final1/receipt.json) intentionally retains
`success=false` and that classification; it is never rewritten as wrapper0.

Only the runner's cleanup-classification code changed afterward. The old runner
bytes are preserved in [`runner-during-checks.py`](final1/runner-during-checks.py)
and match their original pre-start SHA. The new helper is tested by inspecting
only those same exact owned resources and observing their genuine absence;
[`postcleanup-audit.json`](final1/postcleanup-audit.json) records actual inspect
exits/output, wrapper1, the bounded fix and no test-suite repeat. The other18
source/test/config inputs still exactly match the persistent original
[`prestart-source-sha256.json`](final1/prestart-source-sha256.json). The current19
input hashes are separately pinned in
[`postcleanup-source-sha256.json`](final1/postcleanup-source-sha256.json).
No runtime or test code changed after final checks.

All26 original logs, including genuine failures and the five final check logs,
are stored losslessly as `.log.gz`. The
[`lossless-log-manifest.json`](lossless-log-manifest.json) gives original,
compressed and decompressed roundtrip SHA256 and byte counts. No separate raw
`.log` copies remain. Final fresh independent GPT-6.1 Sol HIGH reviews found
**0 required findings** on both axes: [Standards](standards-review.md) had one
optional P3 checkpoint-type maintenance suggestion, and [Spec](spec-review.md)
had zero optional findings. Both reviewed the same44 staged paths and verified
source, log and cleanup provenance. The final46 paths add their verbatim reports
and observed prose; no runtime, test or runner change followed their reviews.

Native Linux loopback, real Ed25519, SQLite, iroh TLS/QUIC and actual LogSync.
WASM checking establishes compilation only. No physical power-loss, new NAT
probability, Windows/Android/browser runtime, secret rotation, migration,
original-code takeover, UI, service ACL or TUN claim follows from these checks.
Frozen prior NAT evidence and executables remain untouched.
