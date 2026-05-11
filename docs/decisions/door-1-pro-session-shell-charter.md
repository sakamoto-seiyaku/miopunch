# Door 1 Pro 会话壳纲领（当前桌面主线）

## 文档状态

- 本文档固化当前 Door 1 Pro 桌面主线的目标、边界与关键决策。
- 本文档只收口当前阶段的产品形态，不展开实现细节，也不替代后续 OpenSpec change。
- 当前主线以“单入口 + 用户态会话 daemon + 非管理员真机验证”为准；若与 `docs/decisions/door-1-client-shell-charter.md` 中的 installer-first / system-daemon 默认交付口径冲突，以本文档为准。
- `docs/decisions/door-1-client-shell-charter.md` 保留为后续 privileged 路线的事实源，用于承接安装器、system service、root/管理员权限与未来虚拟组网相关能力。

## 背景

- Alpha/POC 已完成最小闭环：`miopunch up`、`LocalAPI`、task/report、`sh_attach` 都已经存在。
- 当前主要矛盾不在“还能不能继续扩协议/扩打洞”，而在“能不能快速形成可测、可演示、可自动化的客户端形态”。
- 现有 installer-first / system-daemon 路线把桌面端验证绑定到 root/管理员权限、安装器流程与后续虚拟组网预期上，导致真机 smoke 和自动化推进成本过高。
- 当前阶段的目标是尽快把桌面 Pro 形态收口成“打开即用”的客户端；虚拟组网、TUN、system service、安装器等特权能力明确后移。

## 核心决策

### 1) 当前主线是 session-first，而不是 installer-first

- 当前桌面 Pro 形态采用 `session-first`：
  - 用户入口只有 `miopunch-desktop`。
  - 默认交付先以便携 bundle / 绿色软件式验证为主，而不是 `NSIS` / `.deb` 安装为主。
- 当前阶段不把“先安装 system service / 先完成管理员提权”作为客户端可用的前置条件。

### 2) 单入口：不再要求用户先手动启动 daemon

- 用户不应再经历“先运行 `miopunch up`，再打开 GUI”的两步式流程。
- `miopunch-desktop` 是唯一默认入口：
  - 若本机已有可用 `LocalAPI`，GUI 直接复用。
  - 若 `LocalAPI` 不可达，GUI 负责无感拉起或恢复同用户会话内的 daemon，再完成连接。
- 对用户而言，当前阶段的正确心智模型是“打开 App 即用”，而不是“先管理后台进程”。

### 3) daemon 运行方式：每用户、会话态、非管理员

- 当前主线 daemon 运行在用户会话内，而不是 system service。
- GUI 与 daemon 仍只通过 `LocalAPI`（unix socket / named pipe）交互，不新增产品默认的对外网络监听。
- 当前阶段不把 root/管理员、operator group、stable binary path、system-wide install 作为日常使用前提。

### 4) 窗口与常驻语义

- Windows：
  - 单实例。
  - 关闭窗口默认不是退出，而是隐藏。
  - 允许缩到任务栏/托盘并继续常驻。
- Linux：
  - tray 优先。
  - 若桌面环境支持 tray，则关闭窗口隐藏并继续常驻。
  - 若桌面环境不支持稳定 tray，则关闭窗口直接退出，避免“窗口消失但无入口恢复”。

### 5) 技术边界

- 继续坚持 `LocalAPI-only`：客户端与 daemon 的事实边界不变。
- 继续排除 `Electron`。
- 允许依赖系统 WebView / WebKit 运行时；当前阶段不追求单文件 all-in-one。
- 交互式 `sh_attach` 终端仍然是桌面端必备能力，不因路线转向而降级。

## 当前阶段目标与验收口径

### 当前目标

- 先把桌面 Pro 形态做成“打开即用”的真机可验证客户端。
- 优先覆盖 `Windows / Linux` 桌面端。
- 优先解决：
  - 单入口启动
  - 单实例与窗口恢复
  - 会话态 daemon 生命周期
  - 托盘/隐藏/退出语义
  - 真机 smoke 与自动化阻力

### 当前验收口径

- Windows：
  - 普通用户可直接打开 `miopunch-desktop`
  - GUI 能自动连上或拉起同用户 daemon
  - 支持单实例与隐藏常驻
- Linux：
  - 非 root 用户可直接打开 `miopunch-desktop`
  - GUI 能自动连上或拉起同用户 daemon
  - tray 可用环境下支持隐藏常驻；无 tray 环境下可安全退化为关闭退出
- 两端都以“桌面会话体验”作为当前 smoke 标准，而不是以 `NSIS` / `.deb` 安装成功作为当前通过标准。

## 明确延后的能力

- `NSIS` / `.deb` 安装器主线。
- system service / 开机自启 / stable binary path 对齐。
- Linux `miopunch-operators` 组与对应权限治理。
- Windows 管理员安装、Linux root 安装。
- TUN / 虚拟组网 / 未来需要特权权限的网络栈能力。
- 任何把“当前客户端可用性”重新绑回安装器或特权权限的要求。

## 与旧 Door 1 纲领的关系

- `docs/decisions/door-1-client-shell-charter.md` 仍然有效，但其当前职责已收敛为：
  - privileged 安装路线参考；
  - system daemon / packaging / 安装器口径参考；
  - 后续把会话态产品补齐为系统托管产品时的延伸基线。
- 当前阶段推进桌面 Pro 主线、更新 `docs/roadmap.md`、以及后续创建实现 change 时，应优先引用本文档。
