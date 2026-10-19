# M104 two-axis review

Fixed point: `d1b873e93797a6bbe228eed108a09d44600f249c`.
Final implementation: `26a107d1c0f415bd56744f614adfb64f236855c7`
(main `7522ee46`, metadata-lock clarification `54a55b83`, threshold correction `26a107d1`).

## Standards

0 hard standard violations; 0 actionable heuristic findings; 0 unresolved
documentation accuracy findings. The initial review found one factual error:
Native proof-map 512 is checked after insertion and only marks observer loss,
so it is a qualification threshold rather than a retained-map or heap cap.
The note now states this explicitly, distinguishes the actual queue/core/file
budgets, and retains metadata-mutex contention costs. The reviewer confirmed
closure against the unchanged production code. The initial report is preserved.

## Spec

0 remaining findings. The reviewer checked ordinary original/fork baselines,
actual off/on/finish and real-driver callback failures, capture budgets,
Native/WASM input/feature evidence, 28 outer tests with ten diagnostic child
cases, unchanged SDK pins and full current SDK behavior. The corrected proof
threshold wording satisfies the capacity contract.

Both reviews wrote reports only, without rerunning tests/builds or mutating
source, Git, Plane, cleanup or private SDK evidence.

Final findings: Standards 0; Spec 0. One initial documentation error corrected.

## Standards report

# MIOPUNCH-104 Standards review

Reviewed all four paths in
`d1b873e93797a6bbe228eed108a09d44600f249c...26a107d1c0f415bd56744f614adfb64f236855c7`
(three commits), including the documentation-only follow-up.

**Hard documented-standard violations: none found.** No production Rust or
features changed. Existing explicit installation and ordinary behavior remain
consistent with ADR 0012. Sampled build guards, exact selector lock continuity,
shared target and retained SDK selected-input qualification follow `AGENTS.md`
and `docs/agents/build-cache.md`.

**Actionable heuristic smells: none found.** Per-process cases are required by
the existing once-only observer. Their mode-specific setup and assertions do not
warrant a new production abstraction or strategy hierarchy.

**Prior documentation finding: closed.** The compatibility note now correctly
describes 512 as a post-insertion proof-map qualification/loss threshold. It
explicitly states that exceeding it increments `observerLost`, does not roll
back the entry or stop the core observer, and does not impose a hard retained-map
or heap cap. It separately identifies the bounded writer channel, file-writing
limit and core record/field budgets. This matches the unchanged implementation;
no documentation accuracy findings remain outstanding.

The mutex contention clarification accurately describes blocking
`Mutex::lock`, metadata/proof work under capture budgets, and absence of SDK
calls, I/O or await inside that critical section. The core record/field budgets, finish
semantics, ring/module distinction and physical-send proof limitations match
the inspected source.

Sanitized evidence records 28 outer tests, ten diagnostic child cases and a
qualified whole SDK run. The later documentation-only guard change preserves
the exact earlier SDK snapshot and selected inputs. No builds, tests, cleanup,
source edits, Git/Plane mutations or delegation were performed; only this report
was written. Private SDK material and the 249 protected WIP were untouched.

## Spec report

## Spec

No Spec findings.

Reviewed `d1b873e9...26a107d1` against MIOPUNCH-104, its M91 parent and ADR 0010/0011/0012. The four-file change adds actual public-observer regressions and contract documentation without changing production Rust, permission policy, default features or scheduling.

The requirement “安装 observer 才产生声明的诊断记录；明确关闭、容量及 callback 契约” is covered by ten fresh-process cases, including ordinary UDP/TLS streams, finished capture, actual-driver recursive/panic callbacks, bounded-loss cases and real packet-gate refusal. The sanitized receipts report 28 outer checks passing, plus unchanged original/fork consumer baselines and a complete current Native/Browser SDK approval, durable save, same-identity reopen, PONG and natural cleanup.

The documentation meets “记录与 hash／copy／allocation／lock 工作按启用状态标记”: it separates residual disabled work from enabled costs, explains the Native proof-mutex contention, and distinguishes field counters from allocator/CPU measurements and physical-send evidence. Retaining runtime `install`/`finish` satisfies the explicitly conditional diagnostic-feature criterion; Native/WASM actual inputs establish that `doc(hidden)` does not isolate compilation.

The final wording correction was checked against `Proofs::observe`, `Proofs::transmit` and `observe_record`: 512 is a post-insertion loss/qualification threshold, with no rollback or core-observer stop. The note now states this explicitly and separately identifies the bounded writer queue, core capture budgets and file-writing limit. This resolves the earlier factual capacity claim without introducing a production change.

Limits: read-only source and sanitized-evidence review; no independent build/runtime rerun or private SDK-log inspection. Callback contracts prohibit SDK-lock reentry and waiting for SDK progress; recursive `emit` protection is accurately described as narrower. Existing metadata-mutex contention is disclosed, with no claim of a wholly nonblocking callback or timing equivalence. Public, physical-device and full-platform matrices remain outside this ticket.
