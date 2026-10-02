# MIOPUNCH-52 independent Spec review

Reviewed `17b508576bebb8a382761641a99b02a40e4c64b3...310a6f841ecadb984e6caaf17313594355b6445d` against the full current MIOPUNCH-52 ticket, parent MIOPUNCH-14, blocker 35/38 descriptions and completion comments, current context/design and relevant ADRs. Inspected CLI/core changes, reproduction scripts, delivery note, runtime results/logs and affected-check evidence. No tests rerun or source changes made.

**Outcome: 0 required findings; 0 optional findings.**

- **Missing/partial required behaviors:** None identified within ticket 52. The shared-core CLI exposes creation, manual/precode invitation and joining, daily management, genesis elevation and source-scoped LogSync. The 11 recorded runtime scenarios cover a Windows/Linux QUIC round trip and both directions of creation/joining, pending-material restart, management, restored precode counts, and persisted resource restrictions with unreachable/backoff/recovery outcomes. Distinct permission, elevation, pending-material and unreachable results are visible; count trust is exposed locally without leaking invitation validity to remote applicants.
- **Unrequested behavior/scope creep:** None identified. Backup/restore and the pause-before-save fixture directly support acceptance. Additional precode ALPN selection, Windows file publication and removal of unused cdylib artifacts are documented integration adjustments; they do not introduce UI, ACL, TUN, NAT algorithms or new authorization semantics.
- **Implementation appears wrong:** No concrete spec violation identified in the reviewed changes. The CLI delegates signing, authorization, admission completion, resource validation and persistence to the shared core.

Platform qualification follows the exact acceptance clause: “真实平台或等价受控执行环境中完成核心管理闭环，记录未覆盖的平台限制，不用仅交叉编译代替运行证据。” Evidence executes the actual Windows PE artifact under Wine 10 with real crypto, TLS/QUIC and SQLite. It establishes controlled-runtime coverage, not native Windows kernel, NTFS, ACL or physical-power-loss certification; these limits are explicitly recorded. No additional platform gate is inferred.

The evidence accurately retains the initial expiry-fixture regression failure, isolated pass and corrected full-suite pass, and distinguishes five inert crash-worker entries in the reported 121 entries. Unchanged slow NAT checks need not be repeated for this scope.
