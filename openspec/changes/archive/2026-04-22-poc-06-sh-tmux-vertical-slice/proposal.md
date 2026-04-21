## Why

当前仓库已在 POC-05 完成 `miopunch up` + LocalAPI v0 的骨架，但 `invite/join/approve/ping/sh_ls/sh_attach` 仍为 stub，导致 POC 验收闭环 `join → ping → sh(tmux)` 无法跑通，也无法验证“远程 Shell + tmux 现场恢复 + 可解释性”这条主线。

因此需要在 POC-06 交付 `sh(tmux)` vertical slice：实现可交互的 `miopunch sh`，冻结单写者锁与现场语义，并覆盖 Windows（WSL/SSH targets）与 Linux（local target）两类被控端形态，为后续 POC-07 面板与可解释性报告导出提供可复用地基。

## What Changes

- 落地 `sh` 远程 Shell（tmux 现场）最小实现：
  - `miopunch sh ls`：列出 targets（以及按 target 列 sessions）
  - `miopunch sh`：进入/恢复对端现场，语义固定为 `exec tmux new -A -s <session>`
- 冻结 targets/连接器（POC v0）：
  - Windows 被控端：`wsl:<distro>`（ConPTY + `wsl.exe`）、`ssh:<name>`（系统 `ssh`）
  - Linux 被控端：`local`（本机 PTY + tmux）
- 冻结单写者锁（POC v0）：
  - 同一 `(peer,target,session)` 同时只允许 1 个控制端 attach
  - 锁保活与 WebSocket 活动绑定；超过 TTL 无活动自动释放，避免崩溃/断网导致永久占用
- 冻结 LocalAPI `miopunch.sh.v0` WebSocket I/O 语义：
  - binary frame：PTY 字节流透传（stdin/stdout）
  - text frame：控制 JSON（POC 最小：`winsize{cols,rows}`）
- 打通 POC 端到端最小闭环（验收边界）：`join → ping → sh(tmux)`（失败输出保持 `stage + reason_code + facts + suggestions`）

## Capabilities

### New Capabilities
- `miopunch-poc-shell-v0`: 定义 `sh_ls/sh_attach`、targets（Windows WSL/SSH + Linux local）、tmux 现场语义、单写者锁、以及 `miopunch.sh.v0` 帧语义与最小 reason_code 集合

### Modified Capabilities
- (none)

## Impact

- Affected code (expected):
  - `cmd/miopunch`: `sh` 交互（raw 模式、resize、ws attach）与 `sh ls` 输出
  - `internal/localapi`: `GET /api/v0/tasks/<task_id>/ws` 的 WebSocket 实现（I/O 转发、ping/pong/保活）
  - `internal/task`: `sh_ls` / `sh_attach` task runtime、单写者锁与报告输出
  - 连接器实现（Windows WSL/SSH、Linux local）与相关单元/集成测试
- Dependencies (expected):
  - 可能引入少量 PTY/终端相关依赖（按平台 build tags 收敛）
- Non-impact:
  - 不引入中心化 data-plane relay；控制面仍允许 mesh-first + MQTT 兜底策略
