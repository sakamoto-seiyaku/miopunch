# 2026-05-13 Desktop Control Console UI Rework Backend Impact

> 状态：设计/影响分析 note。
>
> 背景：承接 `2026-05-11-desktop-control-console-live-refresh.md` 中最后一个 change
> `desktop-control-console-ui-rework`。当前 `cmd/miopunch-desktop/frontend`
> 原型已经基本确认，本 note 先记录它对 LocalAPI、Wails bridge、daemon runtime state
> 和后端数据模型的影响。本 note 不是实现，不是正式 OpenSpec proposal。

## 原型变化概览

这次原型变化已经不只是视觉调整，而是把桌面端从 POC task launcher 推向 control console：

- 一级导航变为 `Network / Shell / Admin / Settings`。
- `Access` 不再作为一级入口。
- 未加入网络或 first-run empty node 默认进入 `Network` 下的 Join 页面。
- `Create invite` 和 `Approve request` 归入 `Admin`。
- owner/admin 才能看到 `Admin`；member 只看到 `Network` 和 `Shell`。
- `Shell` 成为一等入口，不再藏在 peer detail 里。
- `Network` 首页从列表/卡片转为设备拓扑和 selected device panel。
- 拓扑图的 link 必须对齐 node 圆点中心，不能因为 label/按钮布局偏离圆心。
- peer detail 顶部使用两列 card：设备身份、别名/actions；下方保留路径详情和节点元数据。
- peer detail 不再常驻 `Connection health` / metrics card。
- task/fact/stage 仍可保留，但默认应进入 Diagnostics/detail，而不是主路径。

前端测试里的新语义也已经比较明确：

- `/?tab=access` 应兼容重定向到 Network/Join。
- `/?tab=admin` 在非 admin 角色或未加入网络时应回到 Network。
- pending approval request 是 Admin review list 的主路径。
- manual approval code 只保留在 advanced panel。
- Network 的 `Ping` 仍创建 `ping` task。
- Shell discovery 用 `sh_ls` 查 targets；打开 shell 用 `sh_attach`。
- Shell 详情里 target/session 都采用 list-first 模型：target 左键直接打开，live session 左键直接 resume。

## 现有后端基础

当前后端已经具备 control console 的一部分基础，不需要从零开始：

- LocalAPI 已有 `GET /api/v0/desktop/state`。
- LocalAPI 已有 `GET /api/v0/desktop/events` SSE。
- Desktop state snapshot 已包含：
  - `topology`
  - `tasks`
  - `peer_sessions`
  - `shell_sessions`
  - `config`
  - `diagnostics`
  - `approval_requests`
- Desktop state event 已有：
  - `task.upsert`
  - `topology.replace`
  - `peer_sessions.replace`
  - `shell_sessions.replace`
  - `config.replace`
  - `diagnostics.replace`
  - `approval_requests.replace`
- Wails bridge 已有：
  - `DesktopRuntimeStart`
  - `DesktopRuntimeResync`
  - `SaveDesktopConfig`
  - `CreateTask`
  - `GetTask`
  - `ExportDiagnostics`
  - terminal bridge info
- Settings 已有 desired/effective/apply 基本结构。
- pending approval requests 已能从 invite store 进入 desktop state。
- shell session summary 已能从 running `sh_attach`/`sh_ls` task 派生。

因此后续重点不是新增一套前端专用 API，而是补齐现有 desktop state contract，
让 GUI 能用真实 runtime snapshot 渲染原型中的 control console。

## 需要补齐的 contract

### 1. 桌面本地别名

原型里的 `Local alias` 不能只放在前端内存里，否则刷新/重启后会丢失。

建议：

- 在 `DesktopPreferences` 增加 `peer_aliases`：

```json
{
  "peer_aliases": {
    "peer-livingroom-mini-03": "Media Box"
  }
}
```

- `PATCH /api/v0/desktop/config` 接受 `preferences.peer_aliases`。
- `SaveDesktopConfig` 返回的新 snapshot 立即包含别名。
- 别名是 desktop-local preference，不写入 governance/decls，不同步给其他节点。
- Peer ID 和 remote member name 永远保留，不能被 alias 替换。

### 2. 成员显示名

原型用 `display_name/device_name/member_name/name` 作为 remote device name。
后端目前 `TopologyMember` 只暴露 `peer_id/role/v4_hint/v6_hint/revoked`。

建议：

- 从 `approve_member` decl body 中读取并暴露：
  - `member_name`
  - `platform`
- 在 `TopologyMember` 上增加可选字段：

```json
{
  "peer_id": "peer-livingroom-mini-03",
  "role": "member",
  "member_name": "Living Room Mini",
  "platform": "linux"
}
```

- UI 显示优先级建议：
  - local alias
  - remote `member_name`
  - short Peer ID

### 3. Shell Resume 语义

原型里的 Resume 容易被误解成“远端后台 shell 会话持久化”。
当前后端更接近 “running `sh_attach` task 等待或持有一个本地 WebSocket attach”。

建议 v1 语义：

- Resume 只表示恢复前台 WebSocket attach。
- 不承诺远端 shell 后台常驻。
- 不承诺 task 与 local WS 分离后可以长期重连。
- 真正后台 remote shell/session persistence 以后单独设计。

需要补齐：

- `DesktopShellSession` 增加 `attachable bool`。
- `attachable=true` 仅当该 `sh_attach` task 仍可接受本地 WS attach。
- `attachable=false` 时，UI 应显示 existing session 但禁用 Resume，建议用户 Open another。
- 后端 `attachByTask` 当前是 one-shot `sync.Once + wsCh`，后续实现 Resume 时需要显式维护 attach 状态。
- attach 状态变化需要触发 `shell_sessions.replace`。

这部分会触及 goroutine/channel/shared state，后续实现必须按 `$go-concurrency` 约束处理：

- channel size 保持 0 或 1。
- 不在锁内做可能阻塞的 channel send。
- task done 时清理 attach state。
- WebSocket attach 成功/失败/超时都要有明确生命周期。

### 4. 路径详情和 Diagnostics

原型展示了：

- Direct IPv4 / Direct IPv6
- Local endpoint
- Remote endpoint
- Public tuple
- Port
- Punch result

这些字段目前大多是 preview/fallback 数据，不能直接当 live contract。

建议 v1：

- 只暴露能可靠采集的结构化字段。
- 无可靠数据时返回空值，前端显示 `unknown`。
- 不在 v1 做持续 RTT、吞吐、loss 时间序列采样。
- `Ping` 是用户动作，不是 peer detail 常驻状态卡。
- `ping` task 的 `stage/facts/suggestions` 进入 Diagnostics 折叠区。
- 无 recent task 时，peer detail 不显示 Diagnostics card。

建议先给 topology/session 增加可选字段：

```json
{
  "direct_ipv4": "100.92.0.34",
  "direct_ipv6": "fd7a:115c:a1e0::34",
  "local_endpoint": "192.168.31.42:49320",
  "remote_endpoint": "10.0.0.12:55391",
  "public_tuple": "203.0.113.21:49320 -> 198.51.100.91:55391",
  "punch_status": "portmap assisted",
  "port": "55391/udp"
}
```

字段可以先挂在 `TopologyNeighborEdge` / `DesktopPeerSession`，具体落点以实现时最少重复为准。

### 5. Settings apply 语义

Settings 原型已经对 desired/effective/apply 有明确期待。
当前 `DesktopConfig` 已有基础字段，但 UI 需要稳定解释：

- desired：用户保存的期望配置。
- effective：当前 runtime 实际生效配置。
- apply.runtime：
  - `immediate`
  - `future_connections`
  - 后续可扩展为 `restart_required`
- `requires_reconnect` 用来解释 active peer/shell session 是否需要重连。
- `restart_required` v1 先保持 false，除非某项确实无法热应用。

需要避免：

- GUI 直接改配置文件。
- GUI 保存后只改 desired，但 effective 和 apply 不解释当前实际状态。
- data protocol mismatch 这类问题没有清楚的 desired/effective 证据。

## 前端落地注意点

后续前端实现应避免把 preview 数据带入 live 模式：

- `previewDeviceNames` 只用于 static preview。
- live 模式设备名来自 alias/member_name/Peer ID。
- live 模式路径详情只显示后端 snapshot 字段或 `unknown`。
- Network topology link endpoint 应按 node 圆点中心计算；label 不参与几何中心。
- peer detail identity/actions 使用两列 card，移动端折叠为单列。
- `Connection health` / metrics grid 暂时隐藏，不作为 live contract。
- `Ping` button 只触发 `ping` task；idle state 不渲染 Ping card。
- recent task 的 task/stage/facts/suggestions 只在 Diagnostics/detail 中展示。
- `Access` tab deep link 只做兼容跳转，不恢复 Access 一级页面。
- Admin unavailable 时应回到 Network，不展示空 Admin 页面作为主路径。
- task/fact/stage copy 继续存在，但默认收进 Diagnostics/detail。

Shell 页面的建议行为：

- Overview 不渲染 terminal。
- 进入 peer shell workspace 后才渲染 terminal。
- target discovery 只查 targets：`sh_ls { peer_id, target: "" }`。
- peer shell workspace 使用单主工作区：connection bar + live session strip + terminal。
- connection bar 包含 device/current target/session、target input、session input、Find targets、Find sessions、主操作按钮。
- 手动输入 target/session 后直接 `Open`，不再需要 `Add target` 或左右 target/session panel。
- `sh_ls { peer_id, target }` 查 session names；结果进入 session input suggestions，不单独渲染 discovered-name card list。
- live `DesktopShellSession` 只在 compact strip 中显示；点击 attachable live session 直接 resume。
- 当前 target/session 匹配 attachable live session 时，主操作按钮显示 `Resume` 且不创建新 task。
- `attachable=false` 的 live session 保持可见，但点击时报错，不作为主按钮目标。
- `Disconnect` 只在 connecting/connected 且确有 terminal bridge 时出现在 connection bar，不在 idle/disconnected 状态占位。
- idle 不渲染为显眼状态 chip；failed/disconnected/connecting/connected 才显示 compact status。
- layout controls 收敛为 connection bar 内的 compact `Zen`；不再显示 `Hide targets / Hide sessions`。
- Attach 失败后保留 peer/target 选择，允许 retry。
- UI 调整必须配套截图核查：Shell idle connection bar、session discovery suggestions、live session strip、connected/disconnect controls、Network device detail action card 至少各留一张审阅截图。

## 后续 change 建议

建议后续开一个正式 OpenSpec change：

```text
desktop-control-console-ui-rework
```

建议拆法：

1. 先扩展 desktop state/config contract：
   - `peer_aliases`
   - `member_name/platform`
   - `shell_sessions.attachable`
   - optional path fields
2. 再更新 Wails/frontend live mode：
   - 消除 live preview fallback。
   - 让 route/role gating 与新原型一致。
   - 使用新 contract 渲染 Network/Shell/Admin。
3. 最后补测试：
   - frontend Playwright smoke。
   - `internal/task` focused tests。
   - `internal/localapi` contract tests。
   - `cmd/miopunch-desktop` bridge tests。

## 测试建议

Docs-only 变更不需要 full validation。

后续代码实现建议至少覆盖：

- first-run empty node：
  - Network 显示 Join。
  - Admin/Shell hidden。
- member role：
  - Network/Shell visible。
  - Admin hidden。
  - pending approval controls hidden。
- owner/admin role：
  - Admin visible。
  - invite/create 和 approval review 可用。
- alias：
  - 保存后 snapshot/config 包含 alias。
  - resync 后仍显示 alias。
  - Peer ID 和 remote member name 不被覆盖。
- shell：
  - target discovery 创建 `sh_ls`。
  - connection bar 的 `Open` 创建 `sh_attach`。
  - session discovery 只更新 session input suggestions。
  - attachable live session strip item 左键 Resume 且不创建新 task。
  - non-attachable session strip item 保持可见但不能 Resume。
  - websocket close 后显示可恢复/可重试状态。
- path details：
  - 有字段时显示真实字段。
  - 缺字段时显示 unknown。
  - live 模式不显示 preview-only endpoint。
- diagnostics：
  - 无 recent task 时不显示 Diagnostics card。
  - ping 后 Diagnostics 显示 task kind/stage/reason/facts。
- topology map：
  - active/selected/known link 端点与 node 圆点中心对齐。
- shell workspace：
  - `Find targets` 只更新 target input suggestions。
  - `Find sessions` 只更新当前 target 的 session input suggestions。
  - 不出现左右 target/session 常驻 panel。
  - 当前 target/session 匹配 attachable live session 时，主按钮显示 `Resume`。
  - `Disconnect` 不在 idle/disconnected 状态显示。

进入 mainline 的 code-affecting change 仍需按 `$dev` 跑完整 gate。

## 明确非目标

- 不把 Local alias 写入 governance/decls。
- 不把 Shell Resume 定义成真正后台持久远端 shell。
- 不做完整 Settings 编辑器。
- 不做持续 RTT/throughput/loss 仪表盘。
- 不新增另一套 frontend-only state file reader。
- 不让 GUI 绕过 daemon/LocalAPI 直接改运行态。
