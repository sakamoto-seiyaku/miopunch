# MIOPUNCH-33 evidence

Baseline: `b8f40efc9e7e4d2125cfc504e8724c06b53d7a9d`, branch `master`.
Behavior and selected backup boundary: [validation note](../../notes/2026-10-01-precode-validation.md).

Reproduce the final full check in a **new** output directory:

```sh
rtk proxy python scripts/check_precode.py --output /tmp/mio33-rerun-unique
```

The runner creates its own real Mosquitto container, records exact commands,
UTC clocks, exits and twelve source/harness SHA-256 values, requires actual MQTT
capture with no skipped broker tests, and removes only that container.
The frozen original [generation 1 receipt](final-generation1/receipt.json) records
**288 passed, 0 failed, 4 existing ignored**. Full real-broker suite, browser WASM,
default workspace/test typecheck, fault-build typecheck and format check all exited 0.
Sources remained unchanged throughout those checks. Actual shared-delivery MQTT
capture observed **97 publishes**, with no long-lived secret in the invitation
and no independent holder obtaining the original approved result.
Owned container removal and subsequent actual `docker inspect` absence were verified.

The affected [short receipt](short-receipt.json) records **47 passed, 0 failed**:
14 precode, 14 manual admission, 6 daily management, 7 authority and 6 storage.
The short run supplied no broker and explicitly skipped its MQTT fixture;
only the final full run is the real-broker proof.

Individual logs preserve the first missing-API RED and actual single-use restart/retry;
the priority two-independent-TLS-identities last-use competition; revocation RED/GREEN;
fresh-code request namespace RED/GREEN; authority-origin RED/GREEN; supported restore
API RED/GREEN; expiry; actual counter damage RED/GREEN; atomic checkpoint API RED/GREEN;
same-network designated ownership/offline refusal; delayed signed authority LogSync;
actual forged TLS identity; and membership-episode rejection.
The two original regrant fixture failures correctly show rejection at the existing
known-removal connection gate. The corrected fixture observes that boundary, then
checks old-result rejection after a separately signed valid public management regrant.
It does not claim to implement the separate MIOPUNCH-34 lifecycle policy.

All raw logs are losslessly compressed in place. The [packing receipt](lossless-log-receipt.json)
records original, compressed and decompressed SHA-256 values and byte counts for each;
decompressed bytes are exactly the original output, including real failures and progress.
The final runner's original `.log` names correspond to retained `.log.gz` files.
Use `rtk proxy gzip -dc <path.log.gz>` to read them. No previous ticket evidence was changed.

Final independent [Standards](standards-review.md) review found **0 required,
1 optional** maintenance observation (shared timeout constant); independent
[Spec](spec-review.md) review found **0 required, 0 optional** findings. Both used
fresh GPT-6.1 Sol HIGH contexts, reviewed the 42 staged source/evidence paths,
and independently checked source and lossless-log hashes. Only these reports
and this observed review prose were added afterward; no source/harness changed.

Environment: native Linux loopback, real Ed25519 signatures, SQLite, iroh TLS/QUIC,
manual/precode two-material admission and admitted LogSync, plus the owned MQTT broker.
Actual process exits at both quota/grant/result transaction boundaries establish
recoverable commit behavior; physical power-loss testing is not claimed.
The backup test genuinely restores a coherent snapshot taken **before consumption**,
observes its saved count of zero, refuses old-code new redemption, permits a fresh code,
and preserves the stop across normal reopen. No rolled-back trust flag serves as proof.
Arbitrary undisclosed consistent whole-disk rollback and malicious database rewriting
are not detected. Browser validation is compilation only. Broader platform runtime,
new NAT probability evidence, UI, rotation, migration, full recovery, service ACL and TUN
remain outside this ticket. Existing protected user paths remain unchanged.
