## Context

POC-05 已落地 `miopunch up` + LocalAPI v0（HTTP/JSON + SSE + WS stub）以及 task 运行骨架，但核心 POC 命令仍处于占位状态：`invite/join/approve/ping/sh_ls/sh_attach` 没有真实语义，导致仓库定义的 POC 验收闭环 `join → ping → sh(tmux)` 无法跑通。

与此同时，仓库已经具备可复用的底层组件（主要在 P3.5 实验链路中使用）：

- `connectivity/`：收集候选、打洞尝试与可观测性
- `internal/signaling/mqtt`：基于 MQTT 的 visitor↔client 信息交换与同步屏障（无中心 data-plane relay）
- `internal/coordinator`：在无需 coord server 的情况下运行一次性 NAT-hole 分析（`AnalyzeOnce`）
- `dataplane/`：基于 UDP path 的 QUIC/KCP data-plane 建链与流式读写基础

本 change 目标是在不大改底层 punching/connectivity 的前提下，把这些组件“产品化接线”到 `cmd/miopunch` 的 daemon+LocalAPI+task 框架内，并实现 tmux 现场语义、单写者锁、以及 WSL/SSH/Linux 目标连接器的最小闭环。

约束（POC v0，冻结）：

- LocalAPI 仍为 **IPC-only**，并保持 `Host: local-miopunch.localapi` 校验。
- `tmux` 为硬依赖：缺失必须失败并输出可操作建议；不 fallback 到 `screen`/裸 shell。
- 单写者锁以 WS/数据面活动保活，超过 TTL 无活动自动释放。
- `stage/reason_code/exit_code` 稳定字段只增不改（改名需 alias/deprecated）。

## Goals / Non-Goals

**Goals:**

- 落地 `miopunch sh` 的交互最小实现：进入/恢复对端 tmux 现场（`tmux new -A -s <session>`），I/O 字节流透传，resize 同步，Ctrl-C 透传。
- 落地 `miopunch sh ls`：列 targets 与 sessions（以 tmux session 为“现场”）。
- 覆盖被控端 targets：
  - Windows：`wsl:<distro>` 与 `ssh:<name>`
  - Linux：`local`
- 实现单写者锁（同一 `(peer,target,session)` 默认仅 1 个 attach），并给出稳定 `reason_code`（例如 `SH_IN_USE`）。
- 通过 LocalAPI task 机制打通 POC 最小闭环：`join → ping → sh(tmux)`（至少保证失败口径稳定、可解释）。

**Non-Goals:**

- 不实现 `--steal/--force`（后续能力）。
- 不引入中心化 data-plane relay。
- 不实现完整治理链/成员管理/撤销等长期态系统（POC 后续 change 逐步完善）。
- 不做 data-plane 自动协商/降级；仅按配置/默认固定选择。

## Decisions

### 1) State & join code（POC 最小）

- Daemon 维护最小持久化 state（user/system 分离，按运行模式选用），用于保存：
  - 本机“可被连接”的入口（用于被控端）：`proxy_name + secret_key + mqtt broker + topic_prefix + data_proto`
  - 已加入的远端 peers（用于控制端）：按 `peer` 键索引同一份连接信息
- `invite` 默认幂等：
  - 若本机已有可用入口，则重复 `invite` 只复用并重新输出同一份 code（避免无意轮换导致 joiner 失效）
  - 若无入口，则生成随机 `secret_key` 与默认 `proxy_name`（可读性优先、碰撞风险由高熵 secret 覆盖）
- join code 编码（POC v0，最小可实现）：
  - 文本前缀 `miopunch.join.v0:` + `base64url(no-pad)` JSON 载荷
  - `join` 同时接受该格式与 `miopunch://...` URL 形式（便于复制/扫码后置）

### 2) Signaling & data-plane（复用现有组件，最小改动）

- 使用 MQTT broker 作为 signaling（复用 `internal/signaling/mqtt`）：
  - visitor 侧在收集到双方信息后运行 `internal/coordinator.AnalyzeOnce` 产出 `NatHoleResp`（无需 coord server）
  - client 侧等待 visitor 推送的分析结果后进入 punching/建链
- data-plane 默认使用 QUIC（`data_proto=quic`，`quic_cc=bbr`），并保留 KCP 作为可选实现路径。
- 为 shell I/O 增加一个最小 framing（覆盖二进制字节流与 JSON 控制消息），避免在同一通道中混淆数据与控制：
  - `frame.kind=DATA`：PTY 字节流
  - `frame.kind=JSON`：控制消息（最小 `winsize{cols,rows}`；并承载 `sh_ls` 请求/响应与错误）

### 3) Single-writer lock（最小可解释）

- 锁 key：`(peer,target,session)`（POC v0 固定；peer 来自 join code/本地 state）
- 获取时机：在 data-plane 建链后，进入 `CapabilityHandshake` 阶段完成“锁 + tmux 可用性”检查：
  - 锁冲突 → 失败 `reason_code=SH_IN_USE`（`exit_code=6`）
  - tmux 缺失 → `reason_code=SH_TMUX_MISSING`
- 保活与释放：
  - 任意数据面帧（含心跳）都更新 `last_activity`
  - 超过 `ttl=60s` 未见任何活动自动释放
  - WS/数据面断开即释放（优先路径）

### 4) Targets & connectors（平台约束）

- Linux 被控端：内置单一 `local` target
  - `sh_attach`：PTY 启动 `tmux new -A -s <session>`
  - `sh_ls`：`tmux list-sessions -F '#S'`（无 session 时返回空）
- Windows 被控端：
  - `wsl:<distro>`：ConPTY + `wsl.exe -d <distro> -- tmux ...`
  - `ssh:<name>`：ConPTY + 系统 `ssh -tt` 执行 `tmux ...`
  - SSH 认证策略：优先复用系统 ssh 的 key/agent/known_hosts（POC 稳定优先）

### 5) CLI attach（避免污染交互）

- `miopunch sh ...` 的交互通过 LocalAPI WS 完成（`Sec-WebSocket-Protocol: miopunch.sh.v0`）：
  - CLI 进入 raw 模式，stdin→WS binary，WS binary→stdout
  - resize 事件发送 WS text 控制 JSON（`winsize{cols,rows}`）
  - attach 成功后不再向 stdout 打印诊断进度（避免污染交互）；失败信息仍输出到 stderr

## Risks / Trade-offs

- [Windows ConPTY 细节复杂，难以在非 Windows 环境验证] → Mitigation：实现严格 build tags；把 framing/锁等平台无关逻辑做单测；Windows 侧以最小可编译+可读实现先落地，后续在真实设备上补齐验证。
- [公共 MQTT broker 不稳定导致 join/ping/sh 失败] → Mitigation：错误输出必须提示如何换 broker；join code 内显式 pin broker 实例；后续引入配置文件覆盖。
- [单一 peer 同时进行 ping 与 sh 会互相干扰] → Mitigation：POC 先收敛为“同一 peer 同时只允许一个长操作”；冲突输出 `SH_IN_USE` 并提示稍后重试。
