## Why

Linux 端现在已经可以创建网络并生成 join code，但 Windows 端加入失败时，当前日志和桌面错误面仍然缺少足够的 broker / topic / invite 交换事实，导致排查只能靠猜。与此同时，这轮真实机排查需要一份可以持续追加的流水账文档，记录现象、证据、推理和修复动作。

## What Changes

- 为 `invite` / `approve` / `join` 的关键成功与失败路径补充面向排查的非敏感 facts，重点覆盖 `broker_endpoint`、`join_topic`、`reply_topic`、`invite_id`、`network_id`、`peer_id`。
- 保持这些 facts 通过 LocalAPI / desktop bridge 透传到桌面错误卡和 diagnostics 导出，避免 Windows 端只能看到简短 reason_code。
- 新增 `docs/notes` 下的排查流水账文档，专门记录这轮 Windows join 失败的排查过程与结论。

## Capabilities

### New Capabilities

- `windows-join-diagnostics-logbook`: Windows join 失败诊断增强与持续排查记录。

### Modified Capabilities

- `miopunch-poc-invite-join-approve-v0`: enroll 失败诊断面补充 broker/topic/invite 交换事实。
- `miopunch-desktop-gui-v0`: 桌面错误与 diagnostics 承接更完整的 join failure facts。

## Impact

- 计划修改：`internal/pocv1/runtime/*`、`cmd/miopunch-desktop/*`
- 计划新增：`docs/notes/2026-05-29-windows-join-investigation.md`
- 不改变 invite code wire format、approval 语义、broker 选择策略本体；本轮以诊断增强和排查记录为主。
