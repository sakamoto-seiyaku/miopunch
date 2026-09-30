# Spec review — MIOPUNCH-30, final generation 2

**Zero unresolved substantive Spec findings. Original P2 closed; no new required gaps, incorrect behavior or scope creep found.** The initial report remains unchanged at `/tmp/mio30-review-spec-2026-10-01.md`.

Reviewed the final indexed ticket scope against baseline `9ab9bed1786eafca1569b204171e76627d1837a0`, using the originating ticket, complete parent specification, current domain/design baseline and ADRs 0004/0005/0007/0008.

The original P2 concerned “同身份重复请求不重复 Add”. Recovery now precedes every approval's role, membership and deduplication decisions (`src/manual_admission.rs:215`), approved-result validity queries (line 197), server handling (line 316), authorization receipt (line 435) and completion (line 519). Native reopen applies ready committed records before exposing authority (`src/native_network.rs:271–275`). The exact-request branch no longer controls recovery. Tests at `tests/manual_admission.rs:1028` and line 1033 authenticate two request IDs from one actual TLS identity, use the production signed Add/result transaction in an exiting child, and verify both reopened and cached live nodes reuse the original hash, retain two applied records, and finish original-request delivery. Raw evidence 21 reproduces the original failure; 22–24 demonstrate the fix.

“申请端只在两项材料持久保存后报告成功” remains satisfied: signed canonical history is independently replayed and durably applied; complete private secret material is atomically published and fsynced before the credential fingerprint commits and the gate receives it (`manual_admission.rs:476–509`, `native_network.rs:429–464`). Applicant state, not sender SAVE acknowledgement, determines local completion. The authorization-only exit-77 and actual storage-failure tests remain effective.

“错误目标、伪造申请身份被拒绝” and “另一邀请码持有者不能解密或冒充该会话” retain network/contact/TLS binding, current authorization-episode checks and real MQTT isolation coverage. Ordinary grants, shared delivery seams, separate failure stages, and admitted bounded business remain within ticket scope.

Independently checked generation-2 working and indexed SHA values against the receipt and counted raw suite results: **256 passed, 0 failed, 4 existing ignored**, no broker skip; real MQTT capture **101 publishes**. WASM, typecheck and formatting recorded exit zero. Generation-1 evidence is preserved separately. No suite/NAT rerun or source edits performed.

**Spec axis: 0 unresolved substantive findings; 0 optional observations.**
