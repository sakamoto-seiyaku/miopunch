## Context

当前 `POCv1` 的最大缺口不是协议模块，而是产品主线 authority 仍停留在 legacy `task`/desktop state 上。`poc-v1-07-gui-wizard` 当前把 `internal/pocv1/runtime`、parallel LocalAPI v1、六阶段状态机和 GUI 壳都绑在一个 change 里，这会让“先有 headless runtime，再有 GUI 默认入口”的顺序再次倒置。

这次拆分后，`06x` 成为 `POCv1` 当前阶段的真正闭环 change，`07` 则变成纯消费层。最终 POC done 仍由 GUI-led flow 定义，但当前实现 gate 固定为 Linux CLI 真闭环。

## Extraction Strategy

- 新实现进入 `internal/pocv1/runtime`。
- `internal/pocv1/runtime` 是当前 `POCv1` 产品主线的唯一 runtime authority。
- `internal/localapi` 保留名字，但协议层收敛为 Unix socket / named pipe 上的 `JSON-RPC` 控制面和独立流通道。
- `cmd/miopunch` 改为通过 v1 runtime/`localapi` 驱动产品动词，而不是继续直接拼 legacy task internals。
- legacy `internal/task`、`/api/v0/desktop/state` 和 `internal/desktopbridge` 只允许作为 shell/plumbing 或历史兼容参考；它们不再拥有 extracted-v1 的事实源地位。

## Scope

**06x owns:**

- 六阶段 runtime model：
  - `Network`
  - `Enroll`
  - `Discover`
  - `Punch`
  - `SecureSession`
  - `Shell`
- stage transitions
- `SecureSession -> Shell` gate：
  - 至少一次成功 identity-bound `ping` 或 `hello`
- user-facing runtime surface：
  - `stage`
  - `reason_code`
  - `summary`
  - structured `Evidence`
- peer runtime objects：
  - `DiscoverView`
  - peer session lifecycle
  - shell session lifecycle
- CLI action orchestration：
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
- non-interactive CLI contracts：
  - `--format json`
  - `--report`
  - `--redact`
- LocalAPI v1 runtime snapshot / event / action contracts
- same-user shared daemon lifecycle：
  - explicit `up`
  - automatic same-user bootstrap for CLI / GUI when daemon is not reachable

**06x does not own:**

- control-plane wire/enroll/presence/punch/session 领域语义本体（`01/02/03/04/05/06`）
- GUI wizard 呈现、desktop bridge UX、frontend navigation（`07`）
- 旧 `/api/v0/desktop/state`、legacy task action surfaces 与 HTTP/SSE/WS contract 的长期兼容扩展
- system service、tray、packaging、installer
- 完整 Windows CLI 闭环 required gate
- Windows/Linux 真机互连 required gate

## Public Contracts

### Runtime API

`06x` 冻结一条基于本地 IPC 的 extracted-v1 runtime contract：

- `JSON-RPC` over Unix socket / named pipe
- 一组通用 `action + args` 控制调用，覆盖当前产品动词；本 change 不预先冻结完整 JSON-RPC method taxonomy：
  - `init-network`
  - `invite`
  - `approve`
  - `join`
  - `ping`
  - `sh ls`
  - `sh`
  - `revoke`
- 独立的 runtime events 流通道
- 独立的 shell attach 流通道

runtime snapshot DTO 至少包含：

- `stage`
- `reason_code`
- `summary`
- `evidence`
- `discover_view`
- `peer_sessions`
- `shell_sessions`

控制调用失败继续统一返回：

- `stage`
- `reason_code`
- `facts`
- `suggestions`

### CLI Semantics

- `sh` 是同一六阶段 flow 的 shorthand，不是第二套产品模型。
- `sh` 必须自动推进缺失的 `Discover / Punch / SecureSession / ping gate`，只在 gate 成功后进入 shell attach。
- `sh` 保留完整交互语义，包括 shell 生命周期、会话复用和报告导出。
- shell transport 可复用 legacy 壳，但准入、复用、失败映射与最终事实源都归 `runtime`。

## Task Breakdown

1. 先恢复 extracted-v1 product build graph，移除 `cmd/miopunch`、`internal/localapi`、`internal/pocv1/*` 对 missing legacy authority packages 的依赖。
2. 建立 `internal/pocv1/runtime` 的六阶段模型、summary/evidence DTO、reason-code/failure mapping 与 runtime lifecycle。
3. 在 `internal/localapi` 中增加基于 socket/pipe 的 `JSON-RPC` 控制面、runtime events 流和 shell attach 流，且不复用 `/api/v0/desktop/state` 作为 governing contract。
4. 将 `cmd/miopunch` 的产品 CLI verbs 重接到 v1 runtime/`localapi`；保留 JSON/report/redaction 合约，并支持 same-user daemon auto-bootstrap。
5. 接入 `03` 的 `DiscoverView`、`04` 的 `PathResult`、`05` 的 `PeerSession` 与 `06` 的 persist authority，形成 Linux-first 双节点真实闭环。
6. 为 CLI/runtime/LocalAPI v1 增加 focused tests 与 Linux 双节点 smoke 验收。

## Acceptance

- Linux 双节点真实闭环可跑通，这是 `06x` 的 required gate：
  - `up`
  - `init-network / invite / approve / join`
  - `ls`
  - `ping`
  - `sh ls`
  - `sh`
  - `revoke`
- Windows/Linux 真机互连不属于 `06x` required gate，应在 Linux CLI 真闭环后作为后续真实环境 change 补齐。
- 任一失败仍保留 `stage`、`reason_code`、`facts`、`suggestions`。
- `sh` 在未完成 identity-bound `ping/hello` 前不得进入 shell attach。
- `cmd/miopunch`、`internal/localapi` 与 `internal/pocv1/*` 的 product graph 不再因 missing legacy authority imports 失败。
- CLI 与 GUI 后续都以 shared daemon 上的 `localapi` runtime contract 为 extracted-v1 runtime source of truth，而不是 legacy desktop snapshot。
- 迁移方式是一次性切换，不保留长期 `/api/v0` 双栈。
