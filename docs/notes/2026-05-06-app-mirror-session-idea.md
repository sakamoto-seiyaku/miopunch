# 2026-05-06 App Mirror Session 想法记录（临时）

> 状态：随手讨论记录，非 spec，非 roadmap 承诺。
>
> 目的：记录一个远期产品想法，避免丢失语义；后续是否进入主线，需要单独调研、实验分支和 OpenSpec change 决定。
>
> 当前结论：暂不实现，不影响当前 POC / desktop UI / shell 主线。

## 1. 直觉

这个想法不是传统远程桌面，也不是只看一个远端窗口的预览。

更接近的产品直觉是：**由 mio 在远端 peer 启动一个真实 GUI 窗口，本地出现一个可操作的镜像窗口**。

- 远端窗口依旧在远端桌面可见。
- 本地不传输整个远端桌面，只围绕这个由 mio 管理的应用窗口建立 session。
- 本地输入必须同步到远端窗口，像直接坐在远端机器前操作这个应用一样。
- 剪贴板需要同步，至少要支持常见文本内容。
- 音频不是第一优先级，可以后置。

可以暂称为 `App Mirror Session`，也可以在未来 change 中重新命名。

## 2. 和现有能力的关系

现有 `sh_attach` 已经证明了一个模式：

```text
local desktop UI
  -> LocalAPI / loopback WebSocket
  -> daemon task
  -> peer-to-peer dataplane logical stream
  -> remote peer acceptor
  -> remote resource
```

`App Mirror Session` 可以复用这个心智模型，但它不是 shell 的小扩展。它会是一个新的远端交互能力：

- `sh_attach` 的远端资源是 PTY/tmux session。
- `App Mirror Session` 的远端资源是由 mio 启动和管理的 GUI app/window session。

它仍应遵守 miopunch 的边界：不把 MQTT 或其他 signaling backend 当作数据面 relay；媒体、输入和剪贴板都应走 peer-to-peer dataplane。

## 3. 可能的 session 组成

一个完整 session 至少会拆成这些通道：

```text
app_mirror session
  video/control: remote window capture -> local render
  input:         local keyboard/mouse -> remote window/input injector
  clipboard:     local <-> remote clipboard sync
  lifecycle:     launch/focus/resize/close/revoke/disconnect
  audio:         optional, later
```

MVP 如果存在，应该优先考虑：

1. 由 mio 启动远端 app，而不是任意接管远端已有窗口。
2. 只捕获这个 app/window，不捕获整个桌面。
3. 本地输入和剪贴板同步作为核心能力。
4. 音频、文件拖放、多窗口编组、系统托盘等都后置。
5. 每个 session 都有清晰的 peer、命令、窗口身份、权限、审计和关闭语义。

## 4. 参考形态

这些项目或协议可以作为概念参考，但不代表直接依赖或照搬：

- `scrcpy`：最接近“远端仍显示、本地镜像操作”的产品直觉。
- `Xpra`：rootless/seamless remote app、剪贴板、音频等能力值得研究，尤其是 Linux/X11 方向。
- `RDP RemoteApp / RAIL`：可参考远端应用映射到本地窗口的生命周期和输入模型。
- `SPICE`：可参考 display/input/clipboard/audio 多通道拆分。
- `Sunshine / Moonlight / Parsec`：可参考低延迟视频和输入体验，但它们更偏屏幕/游戏串流，不是本想法的完整产品模型。

## 5. 已知难点

- **跨平台窗口捕获**：Linux Wayland、Linux X11、Windows、macOS 的捕获 API 与授权模型不同。
- **输入注入权限**：Wayland/macOS/Windows 都有强权限边界，不能假设可以静默注入。
- **剪贴板同步安全**：必须考虑方向、类型、敏感内容、用户确认和 revoke 后立即停止。
- **窗口身份和坐标映射**：本地窗口尺寸、远端窗口尺寸、DPI、缩放、焦点、鼠标坐标都要定义。
- **生命周期**：本地关闭、远端 app 退出、peer 下线、权限 revoke、dataplane 断开都要收敛。
- **体验目标**：如果延迟和输入反馈太差，它会比远程 shell 更难用。

## 6. 暂定态度

这是一个有产品辨识度的远期方向：比远程 shell 更直观，比完整远程桌面更轻，更适合“从本地管理远端机器上的某个 GUI 工具”。

但它明显超出当前阶段。短期只保留这篇 notes；后续如果要推进，应先做独立调研和实验分支，再决定是否创建正式 OpenSpec change。
