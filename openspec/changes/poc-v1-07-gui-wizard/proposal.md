## Why

当前 GUI 的问题不是“缺信息”，而是“用户不知道下一步做什么”。POC v1 需要一个极简但极友好的 Wizard：按 stage 线性推进，每步最多一个主动作，失败时给出人话解释与下一步建议。

## What Changes

- GUI stage 固定：`Network / Enroll / Discover / Punch / SecureSession / Shell`。
- 输出契约固定：`UserSummary(<=3 lines)` + `Evidence(可折叠/导出)`。
- reason_code 总数上限 12（新增必须合并/替换）。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-gui-wizard`: POC v1 GUI wizard 与输出契约。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：desktop GUI 的状态模型与渲染，移除“信息堆砌式”页面。
