# MIOPUNCH-30 evidence and reproduction

Public entry: `tests/manual_admission.rs`. The baseline is `9ab9bed1786eafca1569b204171e76627d1837a0`. All signing, authorization replay, TLS/QUIC, storage and carrier behavior in these checks use real implementations. The child crash worker deliberately exits without closing its node/store; its normal suite invocation is a no-op without the test-specific environment.

Start an owned broker with the repository configuration, discover its allocated port, and set `MIOPUNCH_MQTT_BROKER` for the checks. This does not require or modify a shared broker:

```sh
rtk proxy docker run --detach --name miopunch30-mqtt \
  --publish 127.0.0.1::1883 \
  --mount type=bind,source="$PWD/scripts/mosquitto-test.conf",target=/mosquitto/config/mosquitto.conf,readonly \
  eclipse-mosquitto:2
rtk proxy docker port miopunch30-mqtt 1883/tcp
rtk proxy env MIOPUNCH_MQTT_BROKER=127.0.0.1:<allocated-port> \
  cargo test --test manual_admission -- --test-threads=1 --nocapture
rtk proxy env MIOPUNCH_MQTT_BROKER=127.0.0.1:<allocated-port> \
  cargo test --workspace -- --test-threads=1 --nocapture
rtk proxy cargo check -p miopunch-browser-probe --target wasm32-unknown-unknown
rtk proxy cargo check --workspace --tests
rtk proxy cargo fmt --check
rtk proxy docker rm --force miopunch30-mqtt
```

The actual dedicated broker was `miopunch30-mqtt`, `127.0.0.1:43264`. Missing broker environment makes the MQTT test explicitly report SKIPPED; the final evidence has the variable set and includes actual ciphertext capture counts. Loopback socket tests need the normal authorized sandbox escalation in restricted environments.

| Evidence | Actual observation |
| --- | --- |
| `01-first-red` | Initial public manual entry missing; compilation RED. |
| `02-first-green-attempt` | CBOR fixture argument compile error, retained. |
| `03-first-green` | Restricted sandbox denied real socket bind; no functional pass claimed. |
| `04-first-green-real-sockets` | Two-node manual approval, two saves, durable reopen and actual member-gated business PASS. |
| `05-priority-auth-only-crash` | Actual applicant exit 77 after authorization only; pending reopen, business denial and original-result retry PASS. |
| `06-wrong-network`, `07-wrong-contact`, `08-forged-applicant` | Actual TLS identity/network target rejection PASS. |
| `09-other-holder-mqtt` | Original cleanup stall; only the owned stalled test process was terminated. |
| `10-other-holder-bounded-red` | Bounded reproduction identified retained test handles preventing carrier destruction, after successful material delivery. |
| `11-other-holder-green` | Handle cleanup fixed; actual MQTT ciphertext and independent-holder result isolation PASS. |
| `12-original-authorization-lifetime` | Removal and fresh re-add do not revive the original request. |
| `13-secret-storage-failure` | Actual material-file failure, pending reopen and original-result retry PASS. |
| `14-corrupt-signed-authorization` | Genuine TLS contact's corrupt signed grant rejected before local persistence. |
| `15-affected-regressions` | 28 PASS: signed authority 7, storage 6, then-current manual checks 10, existing native lifecycle 5. |
| `16-current-grant-reuse` | Another authenticated request for an existing member reuses its valid grant without another Add. |
| `21-approval-commit-recovery-red` | Spec P2 reproduced: real shared approval transaction commits, child exits 78 before application, request B incorrectly adds again (applied 3 instead of 2). |
| `22-approval-commit-recovery-green` | Native recovery before membership/dedup decisions reuses the original committed grant (applied 2). |
| `23-recovery-affected-regressions-generation2` | Recovery variants cover both a reopened node and its still-cached live instance, with affected native/authority/storage/admission regressions. |

`final-check-receipt.json` is the preserved generation 1 receipt: 253 passed, 0 failed, 4 existing ignored; all four commands exited 0. `generation1-source-diff.patch.gz` and its SHA receipt preserve exactly the six-source snapshot validated then. Initial Standards and Spec reports are preserved as `initial-*-review-generation1.md`; the Spec report records the subsequently reproduced recovery P2.

The post-fix full suite, WASM, typecheck and formatting commands are `24`–`27`, with actual exit codes and source SHA-256 in `generation2-final-check-receipt.json`. Each generation records the broker TCP preflight and unchanged source through its checks. The implementation note records scope and limitations. Final independent Standards and Spec re-reviews are retained here when complete.

Generation 2 completed with **256 passed, 0 failed, 4 existing ignored**, real MQTT capture 101 publishes, and all four commands exiting 0. Its affected regressions total 32 passes. [Final Standards](final-standards-review-generation2.md) found zero required fixes and one optional cleanup-wrapper duplication observation. [Final Spec](final-spec-review-generation2.md) found zero unresolved substantive findings and independently closed the original P2. Both reviewers checked working/indexed source hashes and actual raw evidence; neither repeated the full suite or prior NAT experiments. Production/test source remained unchanged after these checks and reviews.

Raw logs are retained losslessly; any `.log.gz` file can be read with `rtk proxy gzip -dc <file>`. Compressed-log receipts, if present, preserve the original byte count and SHA-256. Prior NAT experiments and their evidence were neither changed nor repeated.
