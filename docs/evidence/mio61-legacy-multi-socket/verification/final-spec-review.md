# MIOPUNCH-61 final independent Spec review

Reviewer: GPT-6.1 Sol high, `/root/implement_61_sol61/review_61_core_spec`.
Fixed baseline: `302dd46cfbafdcfb72a89cbe1bcb1cf1db11a94c`.
Scope: MIOPUNCH-61 source, tests, fixture/verifier scripts and retained evidence only. Protected user design-document changes excluded. No source edits, new network trials or full suite reruns.

## Findings

**Zero remaining actionable Spec findings.** The earlier P2 CPU evidence gap is closed by the completed, separately authorized functional supplement described below. CPU was not waived: alignment115 requires CPU recording, but does not unambiguously require isolated CPU measurements for each historical canonical cell. CPU for those original 38 cells remains explicitly `not_measured` and cannot be reconstructed.

## CPU supplement resolution

The root authorized exactly three separately predeclared fresh-process functional measurements, modes0/2/4 index0, without adding natural NAT trials or altering the original38-cell denominator. Reviewed `scripts/legacy_nat_multi_socket_cpu.py`, the ignored real Pair/QAD/TLS/admission/control test, the frozen plan and preflight repair, final raw logs/counters/rusage and the README's scope/limits.

Independent artifact validation passed:

- Plan SHA256 `a5e48c52ec257511bb2480ac1257efafd09be1d191552700d3e40e838c4cd554` matches its freeze marker; all43 recorded source inputs match current files.
- `/tmp/mio61-cpu-test` matches frozen binary SHA256 `206240c1d5123f6f9d417d631f5736b0bd877180430db19e16a77224578c9a46`.
- Exactly three successful child results, no repeats or NAT probability samples; each log and raw-drop counter file matches its recorded hash.
- Actual kernel INPUT-drop totals equal submitted probes: mode0=51, mode2=46839, mode4=2635. Both full cases finish1000 random iterations and256 successful bind attempts; mode4 executes42999..43003 inclusive. Actual phase stops: mode0=5200/5201ms, mode2=38001/38001ms, mode4=38000/38010ms.
- All cases preserve original stable IDs and protected control liveness; FD counts return16→16.
- Actual wait4 user/system CPU seconds: mode0=0.527418/0.058140; mode2=13.649365/5.111731; mode4=2.816101/0.629761.

These are whole isolated two-node debug Rust process measurements, including setup, two QAD servers, TLS/admission, two Tokio workers, control maintenance, executor and diagnostic output. They do not estimate individual peers, production builds, devices, NAT gateways or CPU for the original38 cells. Synthetic functional observations are explicitly labeled; the original normal selector and unchanged executor perform the measured work. The raw-only INPUT-loss fixture is preflighted and counter-verified. Its prior OUTPUT/EPERM correction occurred before any measurement child and is retained.

## Other review results

No additional actionable native-core or evidence findings. Original 38-second deadline and sequential per-listener range execution remain intact. Protected readiness, original QUIC dialing-side initiation, exact physical winner path validation, commit checks and guarded business waits preserve connection/scope/round ownership. Real-IP source pinning, nonce/socket correlation, typed destination-send error continuation, provisional/loser/final-path disposal and physical-versus-report generations, including fallback, retain the agreed contract. Automatic-entry integration remains MIOPUNCH-60.

Independent offline reruns:

- Compressed strict verifier: PASS, all 38 cells; 19 business confirmations, 15 bounded misses, 4 admission rejections; all 19 confirmations nonprimary on at least one peer.
- Corruption verifier: PASS, all 16 deliberate variants rejected.
- All 310 gzip streams independently match both recorded SHA256 values and both sizes: 209450006 original bytes, 54977046 compressed bytes; no mismatches.
- Retained real-broker suite log totals: 236 passed, 0 failed, 3 existing ignored across29 suite summaries; this suite predates the additional ignored CPU test, whose three explicit runs passed separately. Suite not rerun by this reviewer.

Reviewed generation-by-cell provenance, full fixed denominator and RNG/allocator disclosure, preserved runtime REDs, original cell19 harness failure and explicit non-reexecution marker repair, corrected capture-based delay reference, full packet/allocator reconciliation, and source/TTL/winner/feedback checks. Mixed execution generations are explicitly disclosed; results are not represented as 38 final-binary executions or a 38/38 NAT success rate. Linux fixture results do not claim uncontrolled NAT, Android or browser UDP success.

Conclusion: zero remaining actionable Spec findings. The CPU requirement is satisfied by actual separately scoped measurements, with the historical missing metric honestly retained and no changes to NAT success denominators.
