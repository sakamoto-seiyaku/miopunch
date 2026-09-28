# 2026-05-11 Desktop Peer Status / Observability Gap

> 状态：问题记录，不是修复设计，不是 roadmap 承诺。
>
> 目的：记录当前 desktop/mainline 在“已经能通一次真实数据面交互”之后，仍然存在的状态展示和运行态观测断层。

## Observed Behavior

本次实测环境是 `dist/release/extracted/` 下的 Linux session bundle 和 Windows session bundle。

当前已经确认：

- WSL2 和 Windows 可以 `join`
- 两边在 GUI 中可以互相看到 peer
- `ping` 可以成功

但同时出现了下面这些现象：

- Peer 信息里显示 `IPv4 unknown`
- Peer 信息里显示 `IPv6 unknown`
- Peer 信息里显示 `Path -`
- `Selection` 直接显示内部 reason，例如 `reachability_bucket_rotation`
- GUI 中没有明确的 keepalive / session health 信息
- 现有 `logs/` 中也缺少足够直接的证据来判断 keepalive 是否按预期工作

换句话说，**连通性主链路已经能通，但 GUI 呈现出的 peer/path/health 状态并不能可靠反映真实运行态**。

## Confirmed Facts

### 1. 持久态中的 reachability hint 目前确实是 `unknown`

本次运行目录中的持久化文件已经确认如下：

- Linux:
  - `dist/release/extracted/miopunch_0.0.0-git287017e-first-run-role_linux_amd64_session/data/state.json`
  - `dist/release/extracted/miopunch_0.0.0-git287017e-first-run-role_linux_amd64_session/data/decls/decls.json`
- Windows:
  - `dist/release/extracted/miopunch_0.0.0-git287017e-first-run-role_windows_amd64_session/data/state.json`
  - `dist/release/extracted/miopunch_0.0.0-git287017e-first-run-role_windows_amd64_session/data/decls/decls.json`

这些文件里的 `v4_hint` / `v6_hint` 当前就是 `unknown`。

因此 GUI 里的 `IPv4 unknown` / `IPv6 unknown` 并不是单纯前端渲染错误，而是后端长期状态本身没有被更新成可用 reachability hint。

### 2. 运行时已经拿到更丰富的网络信息，但没有回写成长期 hint

Linux 和 Windows 的 daemon log 都已经出现了 `pocacceptor gather ready`，并且包含：

- `direct`
- `mapped`
- `assisted`
- `tcp_direct`
- `tcp_mapped`
- `tcp_assisted`

这说明运行时 gather 阶段已经拿到了比 `unknown` 更丰富的信息。

但当前没有证据表明这些 gather 结果被稳定回写到 `state.json` 或 `decls.json` 的 `v4_hint` / `v6_hint` 中。结果就是：

- 实时 gather 已经知道得更多
- GUI 和 topology 读取的长期状态仍然停留在 `unknown`

### 3. 真实 path 存在，但 GUI 不一定能显示出来

本次日志已经明确出现真实 path 证据：

- Linux `ping` 成功日志出现 `attempt_path=punching_tcp4`、`path_family=tcp4`
- Windows `ping` / `sh_attach` / `sh_ls` 相关日志也出现 `path_family=tcp4`

这说明“真实路径不存在”不是问题。

问题在于：**真实 path 的事实，并没有稳定进入 GUI 当前使用的 peer detail 数据链路**。

### 4. Topology active edge 的来源与 accept-side session 持有位置不一致

当前 `TopologySnapshot` 中的 `neighbors.active` 来自 `task.Manager.sessions`。

但 accept-side 的 session 主要由 `internal/pocacceptor` 内部的 `peerSessionRegistry` 持有，而不是 `task.Manager.sessions`。

这意味着至少存在一种结构性风险：

- 真实 session 已存在
- accept-side 也正在使用该 session
- 但 topology 汇总里看不到 active edge
- GUI 最终显示 `Path -` 或者只剩下 selected candidate / 内部 reason

### 5. transport/session 层存在 keepalive / idle 机制，但当前缺少稳定可观测性

代码层面已经存在会话健康相关机制：

- QUIC transport 配置了 `KeepAlivePeriod` 和 `MaxIdleTimeout`
- QUIC / yamux session 自身还有 idle closer

因此“完全没有 keepalive 机制”不是当前已确认结论。

当前真正确认的是：

- transport/session 事件虽然有埋点能力
- 但 dial / accept 主路径目前大多没有把这些事件稳定接到可见日志
- GUI 也没有把“最近活动时间 / close reason / active session health”呈现成可解释状态

因此从用户视角看，会得到一个更糟糕的体验：

- 能 `ping`
- 但不知道 session 是否仍然活着
- 不知道是否发生 idle close
- 不知道 close reason
- 也无法从 GUI 直接判断 keepalive 是否按预期工作

## Why This Is Not Just a UI Bug

这次问题不能简单归类为“Peer 页面显示错了”。

原因很直接：

1. 持久态中的 hint 本来就是 `unknown`
2. 真实 path 已经在日志里出现，但 topology 汇总链路没有稳定带到 GUI
3. `Selection` 暴露出的 `reachability_bucket_rotation` 说明内部诊断字段直接泄漏到了用户界面
4. keepalive / session health 的核心问题是缺少观测性，而不是已经证明 transport 功能完全失效

所以这里至少同时包含三类断层：

- **状态持久化断层**：运行时网络信息没有可靠进入长期 hint
- **拓扑汇总断层**：accept-side active session 没有稳定进入 topology active edge
- **观测性断层**：session health / close reason / keepalive 缺少面向用户和 operator 的可见证据

## Follow-up Pressure

这份 note 不提出 redesign，也不提供修复方案。

但这次问题已经明确暴露出一个产品层压力：

- 当前 desktop GUI 更像一个 task launcher
- 还不是一个能够稳定解释系统状态的桌面控制台

后续如果继续推进 desktop 主线，至少需要单独整理：

- peer / path / health 的状态模型
- settings 入口是否足够支撑真实排障
- desktop log、daemon log、task facts、transport events 如何形成统一诊断路径
- GUI 如何区分“已选择目标”“当前活跃边”“最近失败”“会话已关闭”

这些后续工作不在本 note 内展开。本文只记录：**当前 desktop/mainline 已经能完成一次真实 ping，但 peer 状态展示和运行态观测仍然存在系统性缺口。**
