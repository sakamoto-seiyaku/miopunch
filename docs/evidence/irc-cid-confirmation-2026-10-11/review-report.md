# M89 双轴审查

初版 CID 自动条件过宽的发现已修复；两个独立轴分别审查最终13个文件和完成后的真实回执、构件、输入图及证据。原报告均保留。

## Standards

No open documented-standard violations or actionable heuristic smells found in the published evidence and implementation note.

The original eligibility finding remains documented as fixed. All 13 source hashes match the independent final review and published `owned-source-scope.json`; `Cargo.lock` is byte-identical to the fixed baseline. All 26 published probe files match both their manifest hashes and executed originals, including the explicitly tuned Native helper. Native/WASM/test artifacts, build receipts, and 6,590/3,159/5,734 actual dependency-input hashes match their recorded graphs. The 41-test SDK receipt and 432-test Noq raw result agree with the published summaries; all 1,780 Noq inputs remain current. No tests were rerun.

The note and README follow `docs/agents/domain.md` by separating measured evidence from broader product decisions. They distinguish the explicit 60-second caller from default-profile qualification, preserve historical versus current source graphs, limit the 17.4% confirmation gain to two measurements per implementation on the owned ABBA facility, and avoid attributing the overlapping public ranges to confirmation alone. Independently recomputed means are 28,890.8 versus 23,874.7 ms, a 17.3623% reduction.

Public and controlled raw receipt hashes, request times, and qualification flags match their published summaries. Controller source observes actual WSS close events after SDK shutdown and before context destruction. Immediate 500 ms failures remain visible, and the first controlled iteration remains failed. The docs correctly separate nested TLS/hooks/coordinator timings and unequal gate cuts from comparable request-end statistics; repeated frames are not described as unique CIDs.

A scan of 373 actual raw identifier/sensitive values found none copied into published files; no private-key PEM was present. Cleanup proof records host/runtime preconditions and absence after removal of eight generated key files and 24 owned profiles, preserving logs, certificates and pinned evidence as required by `docs/agents/build-cache.md`.

All reviewed source and evidence hashes stayed stable. No product edits or tracker mutations performed.

## Spec

## Spec

**Pass: no open code or evidence findings in MIO-89’s measured scope.** The earlier CID eligibility finding remains resolved; the exact 13 reviewed source hashes are unchanged. Final SDK build inputs, pinned artifacts, copied sources and retained depfiles match their manifests. I independently verified 26 probe-source hashes, current and historical SDK graphs, the 1,780-input Noq graph, raw runtime receipts, byte-identical Cargo.lock, and all 234 protected WIP hashes. Historical CID-only artifacts remain tied to their original compiled inputs and explicitly declared current-source differences.

The requirement “只有证实收益且保留安全及恢复语义的实现才保留” is now supported for both retained changes. CID-only public repeats improve over the fixed baseline. The controlled ABBA comparison holds CID2, facility, TLS and request settings constant: means 28.8908 s versus 23.8747 s, saving 5.0161 s (17.36%); every combined run is faster than either CID-only run. Both controllers differ only in selected SDK paths. All four actual requests retain strict purpose2, the expected TLS peer, restricted handles and a real native pending request.

The final current public story satisfies “初次受限请求、实际测试审批及材料持久保存、两端重开、获准 relay 普通 PING/PONG”: request 43.0806 s under explicit 60 s caller/attempt; durable authorization/secret, identities after reopen, member1 fresh ACK/original binding and ordinary PONG pass. Natural SDK close, zero native drivers and WSS close events precede context destruction. Current six-case regressions, 41 SDK tests and 432 Noq tests pass, including malformed confirmations, cancellation/deadline, CID rotation and eight paths.

The first controlled run remains failed because its 500 ms close observation missed events. The qualified rerun changes only observation after natural SDK closure; its initial false observations remain recorded. Public range overlap prevents isolated public echo attribution or a claimed 22 s net saving. Documentation correctly limits 17.36% to the owned facility and leaves the full default IRC profile uncertified because assembly8s/queue30s overrides remain. These are explicit scope limits, not unfulfilled requirements to certify defaults.

**Axis conclusions:** Spec: pass, zero open findings. Standards: separate independent final report records zero open hard violations/smells; this review does not replace that axis. No tests rerun or product files edited.

Standards：0项未解决；Spec：0项未解决。默认profile认证和独立公网确认收益均未被本轮证据声称。
