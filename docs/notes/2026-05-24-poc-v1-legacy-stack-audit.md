# POC v1 Legacy Stack Audit

## Purpose

本文件把当前“已能跑、但已混杂 v0/v1 语义”的实现标记为 `legacy/v0` 参考面，作为 `poc-v1-01..07` 并行抽离时的事实源。

目标不是立刻删除旧实现，而是先把“哪些可以参考、哪些不再承接新语义”写死，避免 `poc-v1` 再继续往旧栈上补丁式生长。

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
  - 继续可作为桌面壳/IPC/plumbing 复用。
  - 但 `poc-v1-07` 的 stage/runtime 状态机不再以 legacy task manager 为唯一来源。
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
  - `07` 通过平行 `/api/v1/poc/runtime` 消费 extracted-v1 runtime，并把用户面失败压缩到固定 12 个 `UserReasonCode` bucket。
- legacy 包只允许三种用途：
  - 阅读/对照旧行为。
  - 作为叶子机制复用。
  - 作为 desktop/localapi 的薄壳 plumbing 复用。
- 不允许继续在 `internal/controlplane`、`internal/task`、`internal/pocstate` 上直接叠加新的 v1 领域模型。
- 不允许把 “能跑的旧逻辑” 直接等价为 v1 source of truth；v1 的 source of truth 只来自 `poc-v1-01..07`。

## Implementation Order

整体编码顺序固定为：

1. `poc-v1-01-controlplane-wire`
2. `poc-v1-06-persistence`
3. `poc-v1-02-enroll-bootstrap`
4. `poc-v1-03-presence-discover`
5. `poc-v1-04-dial-punch`
6. `poc-v1-05-secure-session`
7. `poc-v1-07-gui-wizard`

每个编号都必须单独可验收；`07` 是最终闭环，不是唯一验收点。
