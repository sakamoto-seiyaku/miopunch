# POC v1 Legacy Stack Audit

## Purpose

本文件把当前“已能跑、但已混杂 v0/v1 语义”的实现标记为 `legacy/v0` 参考面，作为 `poc-v1-01..07` 并行抽离时的事实源。

目标不是立刻删除旧实现，而是先把“哪些可以参考、哪些不再承接新语义”写死，避免 `poc-v1` 再继续往旧栈上补丁式生长。

## Current Audit Result

- 当前 `POCv1` 尚不能独立运行。`go list ./...` 仍因 legacy/missing package import 失败：
  - `internal/task`
  - `internal/pocacceptor`
  - `internal/pocstate`
  - `internal/controlplane`
- 当前 `internal/localapi` 仍是 v0 HTTP/JSON + SSE + WS + task route implementation；它只能作为 shell/plumbing 参考，不能继续作为 extracted-v1 runtime contract。
- `internal/pocv1/runtime` 尚未存在；`06x` 必须先建立 headless runtime、shared daemon authority、localapi RPC/stream contract 与 CLI wiring。
- 外部校准支持 shared daemon 路线：Tailscale 使用 CLI + daemon split（<https://tailscale.com/docs/reference/tailscaled>），ZeroTierOne service 提供本地 API 给 CLI/其他客户端（<https://docs.zerotier.com/api-service/>），NetBird CLI 支持 daemon socket address（<https://docs.netbird.io/get-started/cli>），Wails 提供 Go/JS binding 与 runtime methods 适合作为 GUI bridge（<https://wails.io/docs/howdoesitwork/>）。

## Legacy Mapping

- `internal/controlplane/*`
  - 当前同时承载历史 JSON wire、invite/join/approve、topic 派生、幂等与 mailbox 逻辑。
  - 作为 `poc-v1-01/02/03` 的参考实现来源，但不再承接新的 v1 领域语义。
- `internal/task/*`
  - 当前同时承载产品 task 编排、控制面流程、GUI 状态拼装与 shell 任务。
  - 作为 `poc-v1-02/04/07` 的流程参考，但不再作为 v1 runtime 主干。
- `internal/pocstate/*`
  - 当前同时承载 identity、governance、broker、ui state 等状态落盘。
  - 作为 `poc-v1-06` 的参考，但新的 v1 持久化 authority 改为 `internal/pocv1/persist`。
- `internal/localapi/*` 与 `internal/desktopbridge/*`
  - 继续可作为 LocalAPI / desktop 壳 / IPC plumbing 复用。
  - 但 extracted-v1 runtime contract 先由 pre-07 headless runtime change 拥有；`07` 只消费它。
  - `localapi` 名字可保留，但协议与 contract 必须从 `HTTP/JSON + SSE + WS + task routes` 收敛为 `JSON-RPC over socket + 独立流通道 + runtime authority`。
- `internal/punching/*`, `internal/punchwire/*`, `connectivity/*`
  - 继续可作为 UDP punching 叶子机制参考。
  - `poc-v1-04` 只允许复用“叶子 mechanics”，不允许把 legacy runtime orchestration 原样搬入 v1。
- `dataplane/*` 与 `internal/tlsutil/*`
  - 继续可作为 `PeerSession` / TLS pin / KCP+TLS+yamux 的参考或窄适配层。
  - `poc-v1-05` 不再接受 QUIC/TCP/多 recipe 重新污染 v1 主路径。

## Extraction Rules

- `poc-v1` 新语义默认进入 `internal/pocv1/{persist,wire,enroll,presence,punch,session,runtime}`。
- 当前 `poc-v1` 约束一并固定为：
  - bootstrap 与运行时 broker 都只保留单 broker，不再把 v0 的主备 broker 语义带入主线。
  - trusted member roster 由 `02` 在 `EnrollResponse` 中下发初始 `roster_snapshot`，并由 `06` 持久化。
  - `mailbox_secret` 只用于 `06.TopicScope` 派生 `net_root` / `presence` / `inbox` topic；不参与正文机密性。
  - `03` 的 presence 只承接在线态与展示提示，不再承接 trusted identity / recipient key / inbox authority。
  - pre-07 headless runtime change 通过 `localapi` IPC 拥有 extracted-v1 runtime contract，并拥有最终用户面 `reason_code` surface。
  - 控制面收敛为通用 `action + args` RPC，而不是继续扩写 task-style HTTP route 集。
  - `07` 只消费该 runtime contract，不再拥有 runtime authority。
- legacy 包只允许三种用途：
  - 阅读/对照旧行为。
  - 作为叶子机制复用。
  - 作为 desktop/localapi 的薄壳 plumbing 复用。
- 不允许继续在 `internal/controlplane`、`internal/task`、`internal/pocstate` 上直接叠加新的 v1 领域模型。
- 不允许把 “能跑的旧逻辑” 直接等价为 v1 source of truth；v1 的 source of truth 只来自 `poc-v1-01/06/02/03/04/05/06x/07`。

## Implementation Order

整体编码顺序固定为：

1. `poc-v1-01-controlplane-wire`
2. `poc-v1-06-persistence`
3. `poc-v1-02-enroll-bootstrap`
4. `poc-v1-03-presence-discover`
5. `poc-v1-04-dial-punch`
6. `poc-v1-05-secure-session`
7. `poc-v1-06x-headless-cli-runtime`
8. `poc-v1-07-gui-wizard`

每个编号都必须单独可验收；`06x` 是当前 Linux CLI 真闭环与 shared daemon authority，`07` 是最终 GUI 默认入口收口，不是唯一验收点。
