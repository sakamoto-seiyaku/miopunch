# 2026-05-11 Desktop Control Console / Live Refresh Notes

> 状态：对话滚动记录。
>
> 目的：记录 desktop GUI 从 POC task launcher 走向真实桌面控制台时，需要保留的判断、约束和后续方向。本文不是正式设计，不是 roadmap 承诺。

## 订阅刷新式框架

当前桌面端虽然是 HTML/Wails 套壳，但这不是实时刷新的限制。

后续方向应保持：

- 前端仍作为桌面 GUI 壳，不直接读取状态文件、配置文件或日志文件。
- 后台 daemon / LocalAPI 继续作为运行态和配置态的权威来源。
- 桌面端通过订阅事件获得“哪类状态发生变化”的通知，再刷新或合并对应的权威状态。

推荐的数据流形态：

```text
daemon runtime
  -> LocalAPI event stream
  -> Wails Go bridge runtime.EventsEmit
  -> frontend runtime.EventsOn
  -> invalidate / refresh affected state
```

这类订阅刷新式架构适合 HTML 套壳桌面应用，也符合 Wails/Tauri/Electron 类应用常见做法。

关键点不是替换桌面端与二进制后台进程的交互通道，而是升级交互语义：

- 现有 LocalAPI 不应只暴露 task 创建和 task event。
- daemon 需要把 topology、peer session、pending approval、config、diagnostics 等运行态变化变成可订阅事件。
- GUI 默认不应展示 task/fact/stage 这类内部 POC 细节；这些内容应进入 debug/detail/report。

第一阶段可以采用粗粒度通知：

- `topology.changed`
- `session.changed`
- `approval.requested`
- `approval.changed`
- `config.changed`
- `diagnostic.changed`

GUI 收到这些事件后，可以 debounce 并重新拉取对应快照；后续稳定后再考虑更细粒度 patch。

## 建议优先级：先做可演示闭环

当前不建议先做完整桌面重构，也不建议先铺开完整 Settings / 日志系统。

更合适的目标是先达到一个可真实演示、可稳定运行、可解释状态的桌面闭环：

- 桌面端能正常启动、连接 daemon，并持续显示真实运行态。
- 远程 shell 能在真实网络中可用，例如另一台设备或移动网络下连回自己的 shell。
- 常见 sing / 组网相关设置能在 Settings 中查看和修改，但先只覆盖演示和测试必需项。
- P2P 组网、peer 状态、路径状态、失败原因能被用户看懂，而不是暴露 task/stage/fact 这些内部实现细节。

建议顺序：

1. **运行态刷新和状态模型**
   - 先解决 GUI 不自动更新、peer/path/session health 看不准的问题。
   - 需要让 topology、peer session、pending approval、config、diagnostics 都能通过 daemon 事件触发 GUI 刷新。
   - 否则后面 UI 再漂亮，也还是会手动刷新、状态不可信。

2. **Access / Approval 流程**
   - 把 invite / join / approve 改成正常产品逻辑。
   - joiner 输入 invite 后，admin/owner 端应自动出现 pending request。
   - admin/owner 在 GUI 点 Approve 或 Deny，不应再手动输入同一个 invite code 去启动 approve task。
   - 这一步直接决定多设备演示是否自然。

3. **远程 shell 变成一等入口**
   - 从 peer 页面直接 Open Shell，不要求用户理解 `sh_attach task`。
   - shell 成功、失败、重连、被占用等状态要用用户语言表达。
   - 这是“带着设备现场演示”的核心能力，应早于大规模 UI 美化。

4. **最小 Settings，先覆盖演示必需项**
   - 不先做完整配置编辑器。
   - 优先项：broker/server、P2P network、data proto、默认 target/session、log level，以及 sing 相关常用设置。
   - Settings 应区分 desired config 与 effective runtime config。
   - 保存配置必须走 daemon API：验证、写入、应用或标记需要重启，然后发出 `config.changed` / `restart_required`。
   - GUI 不直接改配置文件，避免状态、配置、运行态不同步。

5. **日志与诊断系统基础版**
   - 日志需要分级：info / warning / error / debug。
   - 默认 UI 展示用户能理解的状态、最近失败原因和建议动作；debug/detail 才展示 task facts、transport events、daemon log、desktop log。
   - 移动网络和多设备测试前至少要有最近失败原因和诊断导出，否则排障会卡住。

6. **最后做界面原型半重构**
   - UI 重构应建立在前面的状态模型和 API 语义稳定之后。
   - 主界面应是“桌面控制台”，不是 task 列表。
   - 重点页面应围绕 Network、Access、Shell、Settings、Diagnostics 组织。
   - task 列表可以保留为诊断/历史，不作为默认用户路径。

排序原则：

- 先修状态源和事件刷新，再修界面表达。
- 先让真实多设备 / 移动网络 / 远程 shell 演示跑起来并且状态可信，再扩展完整配置面。
- 用户动作应逐步抽象成产品 API，例如 Create Invite、Join Network、Approve/Deny Request、Open Shell、Save Settings。
- sing 相关 Settings 需要放在状态与 control API 之后，否则容易变成“改了配置文件但运行态没跟上”的第二套 POC。
- 底层可以继续创建 task，但 task/fact/stage 默认进入 debug/detail/report，而不是主界面。

## 建议 change 拆分

当前这轮 desktop 工作已经足够明确，不建议再开一个巨型 desktop change。

建议拆成下面 5 个 change，并按顺序推进：

1. **`desktop-runtime-state-refresh-foundation`**
   - 范围：运行态状态模型、统一事件、订阅刷新基础。
   - 目标：GUI 不再依赖手动 Refresh 才看到 peer/path/session/approval/config/diagnostic 变化。
   - 说明：这是后续所有 desktop change 的基础，不建议跳过。

2. **`desktop-access-approval-workflow`**
   - 范围：invite / join / approve 的产品工作流重做。
   - 目标：joiner 输入 invite 后，admin/owner 端自动看到 pending request，并直接 Approve / Deny。
   - 说明：这一 change 应先于 Settings 和 UI 大重构，因为它直接决定组网演示是否自然。

3. **`desktop-shell-demo-loop`**
   - 范围：把远程 shell 做成一等入口，隐藏 `sh_attach task` 语义。
   - 目标：从 peer 页面直接 Open Shell，并提供清晰的成功、失败、重连、占用反馈。
   - 说明：这是“带着设备现场演示”的核心能力，建议单独保住范围。

4. **`desktop-settings-runtime-config-and-diagnostics`**
   - 范围：最小可用 Settings、desired/effective config、基础日志分级、最近失败原因、诊断导出。
   - 目标：先覆盖演示和测试必需项，例如 broker/server、`p2p_network`、`data_proto`、默认 target/session、log level、sing 常用设置。
   - 说明：这一 change 不追求完整配置面，而是优先解决真实测试和排障。

5. **`desktop-control-console-ui-rework`**
   - 范围：原型图半重构，主界面从 task launcher 调整为 control console。
   - 目标：围绕 `Network / Access / Shell / Settings / Diagnostics` 组织界面，把 task 列表退到 debug/history。
   - 说明：必须放最后；前面的状态模型、工作流、shell、settings、diagnostics 稳住之后再做，返工最少。

顺序约束：

- 先做 `desktop-runtime-state-refresh-foundation`，否则后续 UI 和 workflow 都会继续建立在不稳定状态源上。
- `desktop-access-approval-workflow` 和 `desktop-shell-demo-loop` 共同构成最小演示闭环，应早于完整 Settings 和 UI 重构。
- `desktop-settings-runtime-config-and-diagnostics` 放在 shell 演示闭环之后，避免先做成第二套 POC 配置面。
- `desktop-control-console-ui-rework` 最后做，避免前置 change 改语义后反复返工。
