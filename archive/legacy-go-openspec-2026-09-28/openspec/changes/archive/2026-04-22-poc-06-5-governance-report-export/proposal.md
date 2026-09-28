# POC-06.5: governance + membership + revoke + report/export

## 背景

POC-06 已交付 `sh(tmux)` vertical slice（WSL/SSH targets + 单写者锁 + 现场语义），但 POC 产品线仍缺少最基本的“治理/成员/撤销”闭环：

- `approve` / `revoke_member` 仍为 stub，无法形成可恢复、可解释、可撤销的最小治理链路。
- report/export 目前仅存在 task report 的内存渲染，缺少可对外分享的导出与落盘保留策略。

这会导致：

- 入网无法做到“可验真 + 可撤销 + 可传播（最终一致）”。
- 即使 `join → ping → sh(tmux)` 可跑通，也无法回答“对端是谁/是否被撤销/为什么拒绝”。
- 不能稳定生成对外分享的“可解释性报告”（report/export）。

## 目标（POC-06.5）

- 将 **治理链/成员管理/撤销** 列为 POC 的基本能力：
  - `invite/join/approve` 形成可运行闭环（含 invite uses/幂等持久化）。
  - `approve_member` / `revoke_member` 进入长期态声明集合（decls，set-union 收敛；revoke 为永久 tombstone）。
  - 最小 `governance/head_snapshot.json`（genesis + 验证口径；owner-only 变更流程后置）。
- 将 **report/export** 前移到 POC-06.5：
  - task 完成后报告落盘（ring buffer 轮转）。
  - CLI 支持 `--report <path>` 导出（可对外输出）。
  - CLI 支持 `--redact` 对外脱敏（最小规则）。

## 非目标（明确后置）

- 完整 owner-signed snapshot 链的 propose/sign/apply CLI（只落地 genesis + 验证口径）。
- 全量 mesh 转发/受限泛洪（POC-06.5 以 MQTT mailbox 为主线，mesh-first 后置）。
- HTTP 面板（移至 POC-07）。

