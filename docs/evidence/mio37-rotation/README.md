# MIOPUNCH-37 validation evidence

The current source generation is **final3**. Its wrapper and all six actual
commands returned 0: three affected public recovery cases passed, with zero
failures/ignored tests; WASM/default/fault typechecks and formatting passed.
All 57 current source/runner hashes match their prestart manifest and archived
source bytes. This is a scoped current-source result, not a claim that the
latest source ran all 31 rotation entries or the earlier 84-test selection.

Reproduce the exact current final selection with:

```
rtk proxy python -B scripts/check_admission_rotation.py --recovery-fixes --output /tmp/mio37-new-verification
```

Use a new directory. `--affected-rotation` selects all 31 rotation entries and
compilation/format checks. Omitting both selection flags also runs the affected
admission/raw-wire/root/NAT checks with an owned MQTT broker. The runner writes
prestart hashes/source bytes before executing, records every actual command
exit, freezes test executables by SHA in OS temporary storage, and checks owned
broker/volume absence when it creates those resources. No broker is needed by
the current recovery-only selection.

## Current verification

Read [final3-summary.json](final3-summary.json), [receipt](final3/receipt.json),
[prestart hashes](final3/prestart-source-sha256.json), each `*-exit.json`,
[source snapshot](final3/source-snapshot.tar.gz), [lossless logs](final3-lossless-manifest.json),
and [reuse basis](final3-reuse-basis.json). The original wrapper source/exit/log
are in `final3/wrapper/`. Final3's runtime diff from final2 is exactly
[one generation-guard hunk](final3-runtime-diff.patch): only an actual pending
predecessor receives catch-up; an already-staged current completion does not
invent an earlier plan. Of 57 final2 inputs, 55 are unchanged; only that module
and the explicit runner selection changed. Current tests cover both sides of
the predicate plus ordinary provider forwarding and its forged-signature denial.
The root case starts with provider generation1, then restores a new member
without that old secret and completes a different initiator's generation2.
It rejects a genuine node-key signature before any history, requires a fresh
original root exporter proof, preserves the original contact/grant/request,
reopens all nodes, and uses new-generation LogSync. Its child actually exited87.

## Prior generations, preserved separately

- **final1:** wrapper/all13 commands actual0; 84PASS/0FAIL/0ignored, including
  all then-current 28 rotation entries (27 behavior cases plus worker), 13 real
  child exits87, affected gate/manual/native/store/daily and both changed raw-wire
  fixtures, original root recovery/rejoin, two credential/scope NAT checks,
  WASM/default/fault/fmt. Its owned MQTT broker captured95 ciphertext publishes;
  the container and two anonymous volumes were removed and inspected absent.
  Independent Spec review subsequently found two uncovered recovery gaps.
- **final2:** after real public RED/GREEN fixes for both gaps, the full31 rotation
  run ended at actualCargo101/wrapper1 with30PASS/1FAIL/0ignored. A stronger
  pending-root generation2 case exposed the erroneous predecessor guard.
  The remaining four compilation/format commands were unstarted. Source/exe/log
  provenance and [failure summary](final2-failure-summary.json) remain intact.
  No broker was created. [final2-reuse-basis.json](final2-reuse-basis.json) records
  the earlier57→54 unchanged/3 changed comparison against final1.
- **incremental:**44 historical runs preserve actual compile/fixture/environment
  failures, genuine public REDs, GREENs and their actual statuses. Byte archives
  start at forged-ACK RED; earlier runs have prestartSHA/rawlogs only. Each
  `generation-note.json` distinguishes this. [The lossless manifest](incremental-lossless-manifest.json)
  checks all raw/compressed bytes/SHA and gzip roundtrips.

The generation summaries do not relabel old passes as latest-source runs.
Read [initial Spec](initial-spec-review.md) (two required P2 findings) and
[initial Standards](initial-standards-review.md) (0required/1optionalP3) alongside
the final [Standards rereview](standards-review.md) (0required/1optionalP3) and
[Spec rereview](spec-review.md) (0required/0optional; both initialP2s closed).
Both independent reviewers verified the bounded current source/selection and
all historical generations. The validation note explains the integration,
admission/material-entry pause inventory and each actual correction.

[nat-selection.json](nat-selection.json) records43 exact prior final1 omissions,
12 unchanged NAT source/test inputs, zero new-name substring collisions and the
frozen MIOPUNCH-60–63 basis. Final2/3 repeat no NAT cases (all45 are excluded in
those selections); the two actual final1 credential/scope passes remain prior
unchanged-path evidence. There are no probability reruns, new denominators or
filtered full-workspace success claims. A scoped rotation-sequencing predicate
change does not modify NAT/gate/manual/root entry implementations.

Logs are stored losslessly as `.log.gz`; each generation's manifest includes
original/compressed lengths/SHA and roundtripSHA. Test executable binaries stay
in temporary storage, outside Git. Native scope is Linux loopback with real
Ed25519, SQLiteFULL, private file/directoryfsync, TLS/QUIC and process exits.
WASM is compilation only. Physical power loss, other-platform runtime,
migration/UI/ACL/TUN/KCP/Brutal/custom congestion and a security audit are not
claimed. See [the integration note](../../notes/2026-10-01-admission-rotation-validation.md).
