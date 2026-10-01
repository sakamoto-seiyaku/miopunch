# Spec rereview — MIOPUNCH-37

Reviewed current final3 WIP against baseline `a6e384ca9d19595721a1b373ebde47feb21efade`, complete original MIOPUNCH-37/parent MIOPUNCH-14, ADRs 0004/0007/0008 and the accepted MIOPUNCH-36 lifecycle. Runtime/tests remained read-only; no new experiments were run.

## Required corrections

**None. Both initial P2 findings are closed.**

The requirement “保留成员重启后仍能使用新秘密” now has a verified catch-up path: [admission_rotation.rs:273](../../../src/admission_rotation.rs#L273) retrieves the exact signed predecessor decision; [line 1088](../../../src/admission_rotation.rs#L1088) matches the locally pending predecessor before committing and activating it. Final3 demonstrates missed completion1, next rotation2, both nodes reopening and successful generation2 LogSync. A later proposal does not imply predecessor completion.

For “分发替换秘密可继续”, a retained original provider can forward another elevated device's exact root plan and completion. Original pending grant/contact and fresh root exporter proof remain required; the initiator checks its TLS peer's own ACK and independently verifies each forwarded node signature, then FULL-commits every ACK before counting ([line 931](../../../src/admission_rotation.rs#L931), [line 943](../../../src/admission_rotation.rs#L943)). Current ordinary/root relay cases reopen all nodes, reject forged forwarded ACKs and a genuine node-key signature before root-history disclosure, preserve original grant/request, and complete new-generation LogSync. The root recipient lacks the provider's generation1 secret and successfully activates generation2.

Current authority/retained-set checks, independent removal, shared durable admission pause, staged-scope restriction, identity/anchor stability and historical-root removal/ordinary rejoin remain intact. No missing required behavior or scope creep identified.

## Optional findings

None.

## Verification scope

Final3's wrapper/all six commands exited0: three affected cases passed; WASM/default/fault typechecks and formatting passed. All57 current/archive hashes and frozen executable SHAs match; lossless manifests and protected files verify. Final2's30PASS/1FAIL/Cargo101 remains historical. Its only subsequent runtime change is the audited generation predicate; current cases exercise both predicate branches and ordinary relay, supporting bounded revalidation. Final1's84 passes/MQTT/NAT checks remain prior evidence for unchanged paths. Unchanged NAT sources/frozen60–63 evidence and collision-free exclusions justify no repeat. Latest full31/84 success, physical power loss and non-Linux runtime are not claimed.
