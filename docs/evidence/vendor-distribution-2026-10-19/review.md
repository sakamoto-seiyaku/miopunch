# M105 two-axis review

Fixed point `090bde3d218897c38098288165d9dc079149591b`; final source
`865e3ac2b96e9a3dd5e37f45e877571154ab20e6` (main `44608104`, boundary
clarification `845a986a`, required-input fix `865e3ac2`).

## Standards

Final hard violations 0; actionable heuristic findings 0. Initial P2 finding:
required root inputs absent before preparation were silently omitted from the
snapshot, allowing Cargo to be invoked before lock failure. Explicit mandatory
root and declared library/build-script checks now precede snapshot creation.
Ten isolated missing-input negatives and six post-prepare mismatch negatives
cover normal and optimized Python; no Cargo marker appears. The current helper
also generated and loaded the expected relay cdylib, with only one leaf rebuilt.
The final reviewer confirmed closure; initial report and source version retained.

## Spec

Final findings 0. The review independently checked restored original bytes,
complete foreign lock rows, default/conditional target boundaries, paired source
packages and actual consumers, explicit Linux cdylibs, preserved root-library
and CLI entries, source inventory, current SDK evidence and scoped exceptions.
The final missing-input delta was also checked and accepted.

Both reviewers were read-only apart from their reports. They did not rerun
builds/tests, touch source/Git/Plane/cleanup, delegate or inspect private SDK logs.

## Standards report

# MIOPUNCH-105 Standards review

Reviewed the 20 owned paths in
`090bde3d218897c38098288165d9dc079149591b...865e3ac2b96e9a3dd5e37f45e877571154ab20e6`
(three commits), including the mandatory-input follow-up.

**Hard documented-standard violations: none outstanding.** The prior P2
mandatory-input finding is resolved. `sources()` now unconditionally requires
the root manifest, lock, configuration, generator/cache modules, both package
manifests and their declared library/build-script inputs. `prepare` collects
this validated snapshot before creating output context. The checks remain
active under optimized Python, satisfying `AGENTS.md` and
`docs/agents/build-cache.md`'s pre-Cargo source/lock validation rules.

The inspected miniature-root controller and ten recorded negatives cover missing
lock, manifest, configuration, library and build script under normal/optimized
Python: no context or Cargo marker is created. Six post-preparation mismatch
cases still fail before Cargo. The latest positive generation is qualified,
with 323 fresh artifacts, one rebuilt leaf, protected artifacts unchanged and
pinning under the shared lease. No new finding arose from this fix.

**Heuristic findings: none warrant reporting.** The generator keeps cache,
source capture and pinning operations together. Restored upstream duplication
and README whitespace are justified by byte-for-byte compatibility/provenance;
they should not be refactored or reformatted for this review.

Read-only comparison confirms all 11 restored files match their pinned originals,
Noq-proto's lock is the exact published file, and Noq's sole changed lock row is
the local companion Proto record. The distribution note clearly distinguishes
source-package content, extracted-pair runtime checks, explicit Linux cdylibs,
root feature selection and unqualified platform/package boundaries. Sampled
receipts cover leaf regeneration after corrupt cache output, preserved SDK pins,
normal cleanup and a distinct nine-fork current audit.

Limits: no builds, tests, cleanup, source edits, Git/Plane mutations or delegation.
Only this requested report was written. Private SDK runtime/logs/keys/stores and
249 unrelated WIP paths were not inspected or modified. Evidence/Plane packaging
was outside this source review.

## Spec report

## Spec

No Spec findings.

Reviewed `090bde3d...865e3ac2` against MIOPUNCH-105, M91, ADR 0010/0011/0012 and the shared-cache rules. The change satisfies “缺失原文件按 pin 来源恢复或作为明确分发边界处理”: all eleven restored files independently match their fixed original hashes; Noq-proto's standalone lock is unchanged, and Noq's only changed package row removes the companion fork's registry source/checksum. Default features and runtime algorithms are unchanged.

The sanitized evidence supports actual standalone examples, the original 50-connection integration test, both benchmark smoke entries, extracted co-distributed package consumers, Linux cdylib generation/loading and the existing relay CLI. Package content checks are explicitly distinguished from compilation and runtime verification. Original example baselines precede the first edit; the fresh complete Native/Browser SDK story uses selected-source-verified retained pins and preserves normal cleanup.

For “不能让混合构件覆盖或错误 Fresh 结果破坏产物可追溯性,” the new generator preserves previous unversioned outputs, regenerates the selected leaf, rejects a Fresh leaf, and pins output plus selected-input provenance under the shared lease. Source/lock/tool/protected-artifact guards run before Cargo, use always-active checks, and have documented normal/optimized-Python negative evidence. The corrupted-cache recovery reproduces the good artifact hash.

The final driver correction explicitly requires root manifest/lock/configuration, helpers and both packages' declared library/build-script inputs before snapshot/output creation. Ten isolated missing-input cases and six post-preparation mismatch cases stop before Cargo in normal and optimized Python; the corrected helper also has positive generation/load evidence. This resolves the missing-before-prepare precondition edge.

The remaining lib-only default is accurately identified under “任何未恢复的旧默认产物说明具体影响、理由与可用迁移／生成入口.” Linux explicit generation is usable without claiming a new ABI or other platforms. Paired Noq package selection, conditional PQ coverage, absent independent iroh/portmapper locks and other unqualified standalone package builds are clearly bounded. The new nine-fork inventory uses fixed provenance while preserving the historical audit and distinguishing source statistics from runtime compatibility.

Limits: read-only source, hash/lock comparison and sanitized-evidence review; no independent builds/runtimes or private SDK-data inspection. No missing requirement, scope creep or incorrect implementation identified within the declared qualified set.
