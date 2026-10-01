# MIOPUNCH-34 evidence

Read [the integration note](../../notes/2026-10-01-revocation-validation.md).

Reproduce individual behavior through `rtk proxy cargo test --test revocation -- --nocapture`.
Reproduce final workspace and broker checks once with:

```
rtk proxy python3 scripts/check_revocation.py --output /tmp/mio34-fresh-evidence
```

The runner creates its own real MQTT broker, records source hashes and raw logs,
and removes only its container. An existing output directory is rejected.

Initial existing first-real-check passed: real business bytes preceded effective
signed removal; original connection closed and new member connection rejected
with unchanged durable admission material. This was an existing-capability GREEN,
not an invented RED. `rejoin-missing-api-red` is the genuine absent public API RED.
`rejoin-first-runtime` is its real new manual approval GREEN. Initial
`causal-lifecycle-green` retained 3 passes and a genuine fixture failure: after
replacing the generated target with a real admitted device the fixture still
attempted duplicate Add; the verifier correctly refused AlreadyAdded. The
corrected causal check passed. Initial affected-short retained 16 passes and
1 expected stale-policy failure: a previous test required rejecting even fresh
restricted requests. The updated test preserves denial of every member/bootstrap
and recovery protocol while checking the deliberate rejoin exception. Old precode
replay now fails in the actual redemption handler, not merely at connection start.

Final verification and independent Standards/Spec review results are appended
once complete. Logs are losslessly gzip with SHA256 and byte-count roundtrip
receipts; all genuine failures and denominators remain.

A later genuine recovery final-stage race required a minimal production fix.
`recovery-stage-race-red` and `recovery-stage-race-compile2` are direct-rustc
artifact mismatch failures, not runtime security failures.
`recovery-stage-cargo-runtime-red` is an initial fixture timeout (QUIC does not
expose a new stream to the peer until data arrives). The corrected gated-dialer
fixture (`recovery-stage-corrected-runtime-red`) establishes the actual failure:
recovery accepted after removal during agreement. `recovery-stage-race-green`
verifies the restored final scope check. Generation1's 293/0/4, MQTT96 and all
five exit0 checks completed with unchanged original source before this fix;
only the later generation's receipt validates the final source.

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
continuous pre-start SHA audit during the original suite. Final reviews verify
these limits independently. Earlier post-fix affected gate/revocation checks
passed23/0, the earlier wide affected short passed67/0, and creator rejoin passed1/0.

Two independent fresh GPT-6.1 Sol HIGH final reviews completed: Standards
**0 required, 1 optional P3 fixture-naming heuristic**; Spec **0 required**.
Both reviewed the same38owned staged paths and independently verified all25
lossless logs, current16source hashes, six compiled test bindings and the
truthful final2 exit/provenance limits. No runtime or harness changes followed.
The final40path commit adds their verbatim reports and observed prose only.
