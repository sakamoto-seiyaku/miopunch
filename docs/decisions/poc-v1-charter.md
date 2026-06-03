# POC v1 纲领（当前产品抽离主线）

## 文档状态

- 本文档固化当前 `POC v1` 主线的目标、边界与关键决策。
- 本文档只收口当前阶段的产品形态与模块边界，不展开实现细节，也不替代后续 OpenSpec change。
- 本文档是后续 `poc-v1` change 重划、`roadmap` 更新与实现收敛的上游事实源。
- 当前阶段若与既有 `poc-v1-07-gui-wizard` 中“07 自己拥有 runtime authority”的口径冲突，以本文档为准；后续应通过新的 pre-07 change 正式重划 `07`。

## 背景

- `poc-v1-01..06` 这一轮的目标不是继续在旧产品栈上补丁式扩功能，而是从当前混杂的 `legacy/v0` 产品路径里抽出一条作者可以完整解释、可以逐段验证、最终可以独立运行的新主线。
- 截至当前，`wire / enroll / persist / presence / punch / session` 已基本形成模块级闭环，但产品级闭环并没有形成：
  - `internal/pocv1/*` 已有较完整的 typed contracts 与实现。
  - `miopunch up`、`LocalAPI`、CLI 命令与桌面入口仍主要依赖 legacy `task` / `desktop_state` / 旧产品拼装路径。
  - `poc-v1-07` 目前同时背负 runtime authority 与 GUI 入口职责，导致“07 看起来像 UI，实际上还夹着产品主线的最后一段”。
- 当前 repo 证据也说明它还不是可独立运行的 `POCv1`：
  - `go list ./...` 仍会因 `internal/task`、`internal/pocacceptor`、`internal/pocstate`、`internal/controlplane` 等 legacy/missing package 引用失败。
  - `internal/localapi` 当前实现仍是 `/api/v0` HTTP/JSON + SSE + WS + task routes。
  - `internal/pocv1/runtime` 尚未建立，CLI/GUI 还没有共同的 extracted-v1 runtime authority。
- 当前阶段的主要矛盾已经不是“协议模块够不够多”，而是“有没有一条不依赖 legacy authority 的 headless runtime 和产品入口主线”。

## 核心决策

### 1) 当前主线先补 headless runtime，再接 GUI

- `POC v1` 当前阶段必须先完成 `headless runtime + daemon + LocalAPI + CLI` 闭环，再让 GUI 成为默认入口层。
- 当前不接受“GUI 先兜底把主线补齐”的路线；否则 runtime authority 会再次与前端壳、桌面状态拼装和旧任务管理器纠缠在一起。
- 因此，`poc-v1-07` 不应再承担新的 runtime authority；它后续只消费前置 runtime。

### 2) 当前阶段的必须通过门槛是 Linux CLI 真闭环

- 当前阶段先按 `Linux-first` 推进。
- 当前必须通过的产品级闭环不是“桌面能打开”，而是 Linux 下真实双节点的：

```text
miopunch up
-> init-network / invite / approve / join
-> ls
-> ping
-> sh ls
-> sh
-> revoke
```

- 上述闭环必须经过真实 daemon、真实 LocalAPI IPC、真实 broker、真实进程边界与真实 shell attach；不允许用进程内 helper 伪造产品行为。
- `Windows` 在这一阶段不是必过产品闭环，只要求后续不阻塞新的主线设计；完整 Windows CLI/GUI 体验与 Windows/Linux 真机互连后移。

### 3) 六阶段产品模型固定，Shell 前必须先过 SecureSession gate

- `POC v1` 的产品阶段固定为：
  - `Network`
  - `Enroll`
  - `Discover`
  - `Punch`
  - `SecureSession`
  - `Shell`
- `SecureSession` 必须先完成一次成功的 identity-bound `ping` 或 `hello`，才允许转入 `Shell`。
- CLI 中的 `join -> ping -> sh(tmux)` 不是另一套产品模型，而是同一六阶段流程的终端 shorthand。

### 4) v1 source of truth 全部前移到 extracted v1 模块

- `POC v1` 新语义默认进入：
  - `internal/pocv1/wire`
  - `internal/pocv1/enroll`
  - `internal/pocv1/persist`
  - `internal/pocv1/presence`
  - `internal/pocv1/punch`
  - `internal/pocv1/session`
  - `internal/pocv1/runtime`
- legacy 包只允许三种用途：
  - 阅读与对照旧行为。
  - 作为叶子机制窄复用。
  - 作为 desktop / LocalAPI / shell transport 的薄壳 plumbing 复用。
- 抽离旧栈时必须显式区分“产品编排噪音”和“算法核心”。例如，
  `internal/task/poc_dial.go` 的 GUI/task/session 拼装可以移出 v1 主干，
  但 XTCP 风格 UDP 打洞依赖的 `connectivity.Gather`、`punchdecision.Analyze`
  和 attempt-ready `NatHoleResp` 决策链路不能被误当成普通 legacy
  orchestration 丢弃。见
  `docs/notes/2026-06-02-pocv1-xtcp-decision-regression.md`。
- 不允许继续在 `internal/controlplane`、`internal/task`、`internal/pocstate` 上直接叠加新的 `POC v1` 领域模型。

### 5) 唯一 runtime authority 属于 `internal/pocv1/runtime`

- `internal/pocv1/runtime` 是当前 `POC v1` 产品主线的唯一事实源。
- 它负责：
  - 六阶段 stage model
  - stage transitions
  - `UserSummary`
  - structured `Evidence`
  - 最终用户面 `reason_code` 映射
  - peer session / shell session 生命周期
  - `SecureSession -> Shell` gate
- `internal/localapi` 只作为 transport/plumbing 壳，名字保留，但协议层收敛为：
  - `JSON-RPC` over Unix socket / named pipe
  - 单独的 runtime event 流通道
  - 单独的 shell attach 流通道
- `localapi` 不再被视为一组 product-facing HTTP routes，而是 extracted-v1 runtime 的本地 IPC 承载层。

### 6) CLI 命令表面延续，底层实现重做

- 当前阶段继续保留产品动词：
  - `up`
  - `ls`
  - `init-network`
  - `invite`
  - `approve`
  - `join`
  - `ping`
  - `sh ls`
  - `sh`
  - `revoke`
- 保留这些命令的目的是延续产品入口心智，而不是保留 legacy runtime 实现。
- `up` 继续保留为显式命令，但它的职责收敛为：
  - 启动/托管同一用户的 shared daemon
  - 暴露 `localapi` IPC endpoint
  - 不再作为旧 task runtime 的事实源
- 其余 CLI 动作默认通过 `localapi` 连接 shared daemon；在 daemon 不可达时，CLI 允许自动拉起同一用户/同一会话的后台进程。
- 当前阶段继续保留：
  - 非交互命令的 `--format json`
  - 非交互命令的 `--report`
  - `--redact`
- `sh` 仍然是交互命令，不要求支持 `--format json`；但建链、gate 与 shell lifecycle 诊断仍要能导出报告。
- `sh` 保留完整交互语义（attach、复用、报告导出与显式 shell lifecycle）；shell transport 可以复用现有壳，但准入、阶段推进与 session reuse 必须改由 `pocv1 runtime` 拥有。

### 7) 07 只收束 GUI，不再拥有 runtime authority

- `poc-v1-07-gui-wizard` 后续只负责：
  - GUI wizard 呈现
  - `cmd/miopunch-desktop`
  - `internal/desktopbridge`
  - frontend / desktop shell
  - 消费 runtime snapshot / events / actions
- `07` 直连 shared daemon 的 `localapi` IPC endpoint，不通过 CLI 进程桥接。
- `07` 不再拥有：
  - `internal/pocv1/runtime` 的最终事实源地位
  - 六阶段状态机本体
  - `SecureSession -> Shell` gate authority
  - 最终的 failure-to-user mapping ownership
- 这意味着 `07` 不是“POC v1 终于能跑起来”的起点，而是“headless 主线已经成立以后，GUI 成为默认入口层”的收口 change。

## 当前阶段目标与验收口径

### 当前目标

- 把 `POC v1` 先做成一条可独立运行、可自动验证、可真实复测的 Linux 产品主线。
- 优先解决：
  - headless runtime authority
  - daemon / LocalAPI v1
  - CLI 命令重接线
  - session gate
  - shell attach 的产品闭环

### 当前阶段必须通过的实现门槛

- Linux 双节点真实闭环必须可跑通：
  - `up`
  - `init-network / invite / approve / join`
  - `ls`
  - `ping`
  - `sh ls`
  - `sh`
  - `revoke`
- 失败路径必须继续满足：
  - `stage`
  - `reason_code`
  - `facts`
  - `suggestions`
- CLI JSON 输出与报告导出必须保留，不能因为重做 runtime 就退化掉自动化与 artifact 能力。

### 最终 POC v1 完整验收

- 当前阶段的必须通过门槛是 Linux CLI 真闭环。
- 最终 `POC v1 done` 的完整验收边界，仍以 GUI-led 六阶段产品流为准，并保持：
  - `Network -> Enroll -> Discover -> Punch -> SecureSession -> Shell`
  - `SecureSession` 内至少一次成功 identity-bound `ping/hello`
  - 不使用 centralized data-plane relay
- 换句话说：CLI-first 是当前实现顺序与 pre-GUI 门槛，不是否定最终 GUI-led 的产品验收边界。

### 设计校准

- 同类产品的主流形态支持当前路线：CLI/GUI 作为入口，长期运行的本机 daemon 或 service 作为 shared backend。
- Tailscale 官方文档把 `tailscale` CLI 与 `tailscaled` daemon 明确拆开，并说明多数 CLI 命令要求 daemon 正在运行：<https://tailscale.com/docs/reference/tailscaled>
- ZeroTier 官方 Service API 文档说明 ZeroTierOne service 提供本地 API，供 CLI 和其他客户端管理本地实例：<https://docs.zerotier.com/api-service/>
- NetBird CLI 文档暴露 `--daemon-addr`，用于把 CLI 请求发往 daemon socket：<https://docs.netbird.io/get-started/cli>
- Wails binding / runtime methods 适合作为 GUI 内部桥接方式；这支持 `07` 作为 presentation/orchestration 层直连 daemon，而不是经 CLI 进程桥接：<https://wails.io/docs/howdoesitwork/>

## 明确延后的能力

- 当前阶段不承诺完整 Windows CLI 闭环。
- 当前阶段不承诺 Windows/Linux 真机互连为必过 gate；该能力应在 `06x` Linux CLI 真闭环之后作为后续 change 或真实环境验收补齐。
- 当前阶段不做对 legacy runtime/task 的兼容适配层。
- 当前阶段不恢复或扩展 centralized data-plane relay fallback。
- 当前阶段不把 HTTP panel 作为新的主线入口。
- 当前阶段不把安装器、system service、tray 语义、桌面 UX 细节与 runtime authority 混在一个 change 内推进。
- 当前阶段不保留长期 `/api/v0` 与新 runtime API 双栈兼容；`06x`/`07` 走一次性切换。

## 与现有文档和变更的关系

- `openspec/specs/miopunch-poc-scope/spec.md` 仍定义最终 `POC done` 的 GUI-led 验收边界；本文档补充的是当前 `POC v1` 的实现顺序与模块边界。
- `docs/notes/2026-05-24-poc-v1-legacy-stack-audit.md` 继续作为旧栈审计事实源；本文档把“哪些必须前移到新 runtime”进一步写死。
- 后续创建新的 pre-07 headless/CLI change 时，应优先引用本文档。
- 后续重划 `poc-v1-07-gui-wizard` proposal/design/tasks 时，应以本文档为上游事实源，而不是继续让 `07` 同时承担 runtime authority 与 GUI 壳职责。
