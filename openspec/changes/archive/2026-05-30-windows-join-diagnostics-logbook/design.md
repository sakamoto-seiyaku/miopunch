## Context

当前 Windows extracted bundle 的 `miopunch.log` / `miopunch-desktop.log` 只有 daemon 启动和状态路径，没有 join 失败的过程证据。runtime 内部 `wrapProblem(...)` 已经能返回 `message`、`error=...` 和 suggestions，但缺少用于区分“打开 signaling session 失败”“publish join_request 失败”“等待 enroll response 超时”的 broker/topic/invite 上下文。

## Scope

本变更只做三类工作：

1. 在 `invite` / `approve` / `join` runtime action 中，为关键成功与失败路径补充非敏感 facts。
2. 确保这些 facts 继续经由 LocalAPI / desktop bridge 暴露给桌面失败卡与 diagnostics 导出。
3. 新增一份 `docs/notes` 流水账，记录 Windows join 排查过程。

不做：

- 不改 invite code 编码格式。
- 不改 broker 自动选择或 embedded broker 广告策略。
- 不把 Windows/Linux 真机互连修好当作本变更的完成条件。

## Runtime Diagnostics Strategy

- `invite` 成功结果继续保留现有 `invite_code`，并明确携带 `invite_id`、`join_topic`、`broker_endpoint`、`network_id`。
- `approve`：
  - signaling session 打开失败时，补 `invite_id`、`join_topic`、`broker_endpoint`。
  - join request 解包后，成功结果补 `network_id`、`invite_id`、`broker_endpoint`、`approved_peer_id`、`reply_topic`。
- `join`：
  - signaling session 打开失败时，补 `invite_id`、`join_topic`、`broker_endpoint`、`network_id`、`local_peer_id`。
  - publish join request 失败时，再补 `reply_topic`。
  - wait enroll response 超时时，保留上述 facts，方便直接判断是 broker 不通、admin 未 approve，还是 reply topic 没回。
  - 成功结果补 `invite_id`、`reply_topic` 与 `broker_endpoint`，便于与失败样本对照。

所有新增 facts 必须避免 secret material：

- 不输出 raw invite code。
- 不输出 mailbox secret、private key、token、invite secret。

## Desktop / Diagnostics Strategy

- 不新增 bridge API。
- 继续复用已有 `BridgeError.Facts`、`runtime-snapshot.json` 和 diagnostics zip。
- 桌面现有错误卡与 runtime evidence 列表已经会渲染 facts，因此实现只需保证 runtime 错误事实能够完整透传。

## Investigation Logbook

- 新文档路径：`docs/notes/2026-05-29-windows-join-investigation.md`
- 风格对齐已有 `docs/notes/2026-05-14-wsl2-windows-connectivity-debug-discussion.md`
- 内容结构：
  - 背景与现象
  - 当前 extracted bundle / logs 事实
  - 代码阅读结论
  - 本轮修复动作
  - 后续待验证项
